use axum::{
    body::Body,
    extract::{Multipart, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::SystemTime;
use crate::common::AppState;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use sqlx::Row;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_unique_id() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let raw = format!("{}-{}", now, count);
    
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let hex = hex::encode(hasher.finalize());
    hex[..16].to_string()
}

pub async fn list_filebox_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT COALESCE(SUM(file_size), 0) AS total_size FROM filebox_files WHERE expires_at >= datetime('now')")
        .fetch_one(&state.db)
        .await;
    
    let used_space: i64 = match row {
        Ok(r) => r.get("total_size"),
        Err(err) => {
            tracing::error!("failed to query used space: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let rows = sqlx::query(
        "SELECT id, file_name, file_size, uploaded_at, expires_at FROM filebox_files WHERE expires_at >= datetime('now') ORDER BY uploaded_at DESC"
    )
    .fetch_all(&state.db)
    .await;

    let files_result = match rows {
        Ok(rs) => rs,
        Err(err) => {
            tracing::error!("failed to list filebox files: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let files: Vec<serde_json::Value> = files_result.into_iter().map(|row| {
        let id: String = row.get("id");
        let file_name: String = row.get("file_name");
        let file_size: i64 = row.get("file_size");
        let uploaded_at: String = row.get("uploaded_at");
        let expires_at: String = row.get("expires_at");
        
        serde_json::json!({
            "id": id,
            "file_name": file_name,
            "file_size": file_size,
            "uploaded_at": uploaded_at,
            "expires_at": expires_at,
        })
    }).collect();

    (StatusCode::OK, Json(serde_json::json!({
        "total_space": state.filebox_size,
        "used_space": used_space,
        "files": files,
    }))).into_response()
}

pub async fn upload_filebox_handler(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT COALESCE(SUM(file_size), 0) AS total_size FROM filebox_files WHERE expires_at >= datetime('now')")
        .fetch_one(&state.db)
        .await;

    let used_space: i64 = match row {
        Ok(r) => r.get("total_size"),
        Err(err) => {
            tracing::error!("failed to query used space: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let mut uploaded_files = Vec::new();

    while let Ok(Some(mut field)) = multipart.next_field().await {
        let file_name = field.file_name().unwrap_or("file").to_string();
        let id = generate_unique_id();
        let file_path = state.cache_dir.join("filebox").join(&id);

        let mut file = match File::create(&file_path).await {
            Ok(f) => f,
            Err(err) => {
                tracing::error!("failed to create file on disk: {err}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Disk error").into_response();
            }
        };

        let mut size = 0_u64;
        let mut quota_exceeded = false;

        while let Ok(Some(chunk)) = field.chunk().await {
            if (used_space as u64) + size + (chunk.len() as u64) > state.filebox_size {
                quota_exceeded = true;
                break;
            }
            if let Err(err) = file.write_all(&chunk).await {
                tracing::error!("failed to write chunk to disk: {err}");
                let _ = fs::remove_file(&file_path).await;
                return (StatusCode::INTERNAL_SERVER_ERROR, "Disk write error").into_response();
            }
            size += chunk.len() as u64;
        }

        if quota_exceeded {
            let _ = fs::remove_file(&file_path).await;
            return (
                StatusCode::BAD_REQUEST,
                "存储空间不足，无法上传该文件。请先清理空间或提高配额。",
            )
                .into_response();
        }

        if size == 0 {
            let _ = fs::remove_file(&file_path).await;
            continue;
        }

        if let Err(err) = sqlx::query(
            "INSERT INTO filebox_files (id, file_name, file_size, expires_at)
             VALUES (?, ?, ?, datetime('now', '+7 days'))"
        )
        .bind(&id)
        .bind(&file_name)
        .bind(size as i64)
        .execute(&state.db)
        .await
        {
            tracing::error!("failed to insert upload metadata to DB: {err}");
            let _ = fs::remove_file(&file_path).await;
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database record error").into_response();
        }

        uploaded_files.push(serde_json::json!({
            "id": id,
            "file_name": file_name,
            "file_size": size,
        }));
    }

    if uploaded_files.is_empty() {
        return (StatusCode::BAD_REQUEST, "没有检测到有效文件").into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({
        "success": true,
        "files": uploaded_files,
    })))
    .into_response()
}

pub async fn download_filebox_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query(
        "SELECT file_name, file_size FROM filebox_files WHERE id = ? AND expires_at >= datetime('now')"
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await;

    let (file_name, file_size) = match row {
        Ok(Some(r)) => {
            let name: String = r.get("file_name");
            let size: i64 = r.get("file_size");
            (name, size)
        }
        Ok(None) => return (StatusCode::NOT_FOUND, "文件未找到或已过期").into_response(),
        Err(err) => {
            tracing::error!("failed to query file metadata: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let file_path = state.cache_dir.join("filebox").join(&id);
    let file = match File::open(&file_path).await {
        Ok(f) => f,
        Err(_) => return (StatusCode::NOT_FOUND, "本地文件丢失").into_response(),
    };

    let mut response_headers = HeaderMap::new();
    let content_disposition = crate::common::build_content_disposition(&file_name);
    if let Ok(value) = HeaderValue::try_from(content_disposition) {
        response_headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    response_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(file_size));

    let body = Body::from_stream(ReaderStream::new(file));
    (StatusCode::OK, response_headers, body).into_response()
}

pub async fn delete_filebox_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let result = match sqlx::query("DELETE FROM filebox_files WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await
    {
        Ok(res) => res.rows_affected(),
        Err(err) => {
            tracing::error!("failed to delete file metadata: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    if result == 0 {
        return (StatusCode::NOT_FOUND, "文件不存在").into_response();
    }

    let file_path = state.cache_dir.join("filebox").join(&id);
    let _ = fs::remove_file(&file_path).await;

    (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response()
}

pub fn spawn_filebox_cleanup_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            
            let rows = sqlx::query("SELECT id FROM filebox_files WHERE expires_at < datetime('now')")
                .fetch_all(&state.db)
                .await;
            
            let expired_ids: Vec<String> = match rows {
                Ok(rs) => rs.iter().map(|r| r.get::<String, _>("id")).collect(),
                Err(err) => {
                    tracing::warn!("failed to query expired files: {err}");
                    continue;
                }
            };

            for id in &expired_ids {
                let file_path = state.cache_dir.join("filebox").join(id);
                if fs::remove_file(&file_path).await.is_ok() {
                    tracing::info!("successfully deleted expired filebox file: {id}");
                } else if file_path.exists() {
                    tracing::warn!("failed to delete expired file from disk: {}", file_path.display());
                }
            }

            if !expired_ids.is_empty() {
                if let Err(err) = sqlx::query("DELETE FROM filebox_files WHERE expires_at < datetime('now')")
                    .execute(&state.db)
                    .await
                {
                    tracing::warn!("failed to delete expired filebox records from DB: {err}");
                }
            }
        }
    });
}
