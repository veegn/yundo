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
use crate::common::{AppState, is_forbidden_host, resolve_file_name};
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use sqlx::Row;
use futures_util::StreamExt;
use url::Url;

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
    let used_space = crate::cache::get_combined_used_size(&state.cache_dir, &state.db).await;

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
        "total_space": state.max_cache_size,
        "used_space": used_space,
        "files": files,
    }))).into_response()
}

#[derive(serde::Deserialize)]
pub struct RemoteUploadPayload {
    pub url: String,
}

pub async fn remote_upload_filebox_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RemoteUploadPayload>,
) -> impl IntoResponse {
    let target_url = payload.url;
    let parsed_url = match Url::parse(&target_url) {
        Ok(url) => url,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid URL format").into_response(),
    };

    if !matches!(parsed_url.scheme(), "http" | "https") {
        return (StatusCode::BAD_REQUEST, "only HTTP and HTTPS URLs are supported").into_response();
    }

    let host = parsed_url.host_str().unwrap_or_default().to_ascii_lowercase();
    if is_forbidden_host(&host) {
        return (StatusCode::FORBIDDEN, "access to local or private networks is forbidden").into_response();
    }

    let initial_combined_used = crate::cache::get_combined_used_size(&state.cache_dir, &state.db).await;

    let upstream_request = state.client.get(&target_url)
        .header("User-Agent", "precision-proxy/1.0");

    let upstream_response = match upstream_request.send().await {
        Ok(response) => response,
        Err(err) => {
            tracing::error!("remote upload request failed for {target_url}: {err}");
            return (StatusCode::BAD_GATEWAY, "failed to reach target server").into_response();
        }
    };

    let status = upstream_response.status();
    if !status.is_success() {
        return (StatusCode::BAD_GATEWAY, "upstream server returned error").into_response();
    }

    let final_url = upstream_response.url().clone();
    let mut response_headers = HeaderMap::new();
    for (name, value) in upstream_response.headers() {
        let header_name = name.as_str().to_ascii_lowercase();
        if crate::common::ALLOWED_HEADERS.contains(&header_name.as_str()) {
            response_headers.insert(name.clone(), value.clone());
        }
    }

    let file_name = resolve_file_name(&parsed_url, Some(&final_url), &response_headers);
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
    let mut stream = upstream_response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                if initial_combined_used + size + (chunk.len() as u64) > state.max_cache_size {
                    quota_exceeded = true;
                    break;
                }
                if let Err(err) = file.write_all(&chunk).await {
                    tracing::error!("failed to write chunk to disk during remote upload: {err}");
                    let _ = fs::remove_file(&file_path).await;
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Disk write error").into_response();
                }
                size += chunk.len() as u64;
            }
            Err(err) => {
                tracing::error!("failed to read chunk from upstream during remote upload: {err}");
                let _ = fs::remove_file(&file_path).await;
                return (StatusCode::BAD_GATEWAY, "network error reading from upstream").into_response();
            }
        }
    }

    if quota_exceeded {
        let _ = fs::remove_file(&file_path).await;
        return (
            StatusCode::BAD_REQUEST,
            "存储空间不足，无法转存该文件。请先清理空间或提高配额。",
        )
            .into_response();
    }

    if size == 0 {
        let _ = fs::remove_file(&file_path).await;
        return (StatusCode::BAD_REQUEST, "转存文件大小为 0").into_response();
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
        tracing::error!("failed to insert remote upload metadata to DB: {err}");
        let _ = fs::remove_file(&file_path).await;
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database record error").into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({
        "success": true,
        "file": {
            "id": id,
            "file_name": file_name,
            "file_size": size,
        }
    }))).into_response()
}

pub async fn upload_filebox_handler(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let initial_combined_used = crate::cache::get_combined_used_size(&state.cache_dir, &state.db).await;

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
            if initial_combined_used + size + (chunk.len() as u64) > state.max_cache_size {
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

fn parse_range(range_header: &str, file_size: u64) -> Option<(u64, u64)> {
    if !range_header.starts_with("bytes=") {
        return None;
    }
    let range_str = &range_header["bytes=".len()..];
    let first_range = range_str.split(',').next()?.trim();
    let parts: Vec<&str> = first_range.split('-').collect();
    if parts.len() != 2 {
        return None;
    }
    let start_str = parts[0].trim();
    let end_str = parts[1].trim();

    if start_str.is_empty() && end_str.is_empty() {
        return None;
    }

    if start_str.is_empty() {
        let suffix_len = end_str.parse::<u64>().ok()?;
        if suffix_len == 0 {
            return None;
        }
        let start = if file_size > suffix_len {
            file_size - suffix_len
        } else {
            0
        };
        Some((start, file_size.saturating_sub(1)))
    } else if end_str.is_empty() {
        let start = start_str.parse::<u64>().ok()?;
        if start >= file_size {
            return None;
        }
        Some((start, file_size.saturating_sub(1)))
    } else {
        let start = start_str.parse::<u64>().ok()?;
        let mut end = end_str.parse::<u64>().ok()?;
        if start >= file_size || start > end {
            return None;
        }
        if end >= file_size {
            end = file_size.saturating_sub(1);
        }
        Some((start, end))
    }
}

fn is_range_out_of_bounds(range_header: &str, file_size: u64) -> bool {
    if !range_header.starts_with("bytes=") {
        return false;
    }
    let range_str = &range_header["bytes=".len()..];
    let first_range = match range_str.split(',').next() {
        Some(r) => r.trim(),
        None => return false,
    };
    let parts: Vec<&str> = first_range.split('-').collect();
    if parts.len() != 2 {
        return false;
    }
    let start_str = parts[0].trim();

    if !start_str.is_empty() {
        if let Ok(start) = start_str.parse::<u64>() {
            if start >= file_size {
                return true;
            }
        }
    }
    false
}

pub async fn download_filebox_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
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
    let mut file = match File::open(&file_path).await {
        Ok(f) => f,
        Err(_) => return (StatusCode::NOT_FOUND, "本地文件丢失").into_response(),
    };

    let file_size_u64 = file_size as u64;

    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

    let content_disposition = crate::common::build_content_disposition(&file_name);
    if let Ok(value) = HeaderValue::try_from(content_disposition) {
        response_headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));

    if let Some(range_header) = headers.get(header::RANGE).and_then(|h| h.to_str().ok()) {
        if let Some((start, end)) = parse_range(range_header, file_size_u64) {
            let chunk_size = end - start + 1;
            if let Err(err) = file.seek(std::io::SeekFrom::Start(start)).await {
                tracing::error!("failed to seek file to {start}: {err}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Seek error").into_response();
            }

            response_headers.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {start}-{end}/{file_size_u64}")).unwrap(),
            );
            response_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(chunk_size));

            let body = Body::from_stream(ReaderStream::new(file.take(chunk_size)));
            (StatusCode::PARTIAL_CONTENT, response_headers, body).into_response()
        } else {
            if is_range_out_of_bounds(range_header, file_size_u64) {
                response_headers.insert(
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes */{file_size_u64}")).unwrap(),
                );
                return (StatusCode::RANGE_NOT_SATISFIABLE, response_headers, Body::empty()).into_response();
            }

            response_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(file_size));
            let body = Body::from_stream(ReaderStream::new(file));
            (StatusCode::OK, response_headers, body).into_response()
        }
    } else {
        response_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(file_size));
        let body = Body::from_stream(ReaderStream::new(file));
        (StatusCode::OK, response_headers, body).into_response()
    }
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

#[derive(serde::Deserialize)]
pub struct UploadCompletePayload {
    pub upload_id: String,
    pub file_name: String,
    pub total_chunks: usize,
}

pub async fn upload_chunk_handler(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut upload_id = String::new();
    let mut chunk_index = 0_usize;
    let mut chunk_data = Vec::new();

    let initial_combined_used = crate::cache::get_combined_used_size(&state.cache_dir, &state.db).await;

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name() {
            Some("upload_id") => upload_id = field.text().await.unwrap_or_default(),
            Some("chunk_index") => chunk_index = field.text().await.unwrap_or_default().parse().unwrap_or(0),
            Some("file") => chunk_data = field.bytes().await.unwrap_or_default().to_vec(),
            _ => {}
        }
    }

    if upload_id.is_empty() || chunk_data.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing chunk metadata").into_response();
    }

    if initial_combined_used + (chunk_data.len() as u64) > state.max_cache_size {
        return (
            StatusCode::BAD_REQUEST,
            "存储空间不足，无法上传分片。请清理空间或提高配额。",
        ).into_response();
    }

    let chunk_dir = state.cache_dir.join("filebox_tmp").join(&upload_id);
    if let Err(err) = fs::create_dir_all(&chunk_dir).await {
        tracing::error!("failed to create chunk directory: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Disk error").into_response();
    }

    let chunk_path = chunk_dir.join(chunk_index.to_string());
    if let Err(err) = fs::write(&chunk_path, chunk_data).await {
        tracing::error!("failed to write chunk to disk: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Disk write error").into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response()
}

pub async fn upload_complete_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UploadCompletePayload>,
) -> impl IntoResponse {
    let id = generate_unique_id();
    let file_path = state.cache_dir.join("filebox").join(&id);
    let chunk_dir = state.cache_dir.join("filebox_tmp").join(&payload.upload_id);

    if !chunk_dir.exists() {
        return (StatusCode::BAD_REQUEST, "No chunks found").into_response();
    }

    let mut final_file = match File::create(&file_path).await {
        Ok(f) => f,
        Err(err) => {
            tracing::error!("failed to create final file: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Disk error").into_response();
        }
    };

    let mut total_size = 0_u64;

    for i in 0..payload.total_chunks {
        let chunk_path = chunk_dir.join(i.to_string());
        let chunk_data = match fs::read(&chunk_path).await {
            Ok(data) => data,
            Err(_) => {
                let _ = fs::remove_file(&file_path).await;
                return (StatusCode::BAD_REQUEST, format!("Missing chunk: {}", i)).into_response();
            }
        };

        if let Err(err) = final_file.write_all(&chunk_data).await {
            tracing::error!("failed to write to final file: {err}");
            let _ = fs::remove_file(&file_path).await;
            return (StatusCode::INTERNAL_SERVER_ERROR, "Disk write error").into_response();
        }
        total_size += chunk_data.len() as u64;
    }

    // Cleanup tmp dir
    let _ = fs::remove_dir_all(&chunk_dir).await;

    if total_size == 0 {
        let _ = fs::remove_file(&file_path).await;
        return (StatusCode::BAD_REQUEST, "合并文件大小为 0").into_response();
    }

    if let Err(err) = sqlx::query(
        "INSERT INTO filebox_files (id, file_name, file_size, expires_at)
         VALUES (?, ?, ?, datetime('now', '+7 days'))"
    )
    .bind(&id)
    .bind(&payload.file_name)
    .bind(total_size as i64)
    .execute(&state.db)
    .await
    {
        tracing::error!("failed to insert upload metadata to DB: {err}");
        let _ = fs::remove_file(&file_path).await;
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database record error").into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({
        "success": true,
        "files": [{
            "id": id,
            "file_name": payload.file_name,
            "file_size": total_size,
        }]
    })))
    .into_response()
}

#[derive(serde::Deserialize)]
pub struct UploadAbortPayload {
    pub upload_id: String,
}

pub async fn upload_abort_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UploadAbortPayload>,
) -> impl IntoResponse {
    if payload.upload_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing upload_id").into_response();
    }

    // Strict validation to prevent directory traversal
    if payload.upload_id.contains('/') || payload.upload_id.contains('\\') || payload.upload_id.contains("..") {
        return (StatusCode::BAD_REQUEST, "Invalid upload_id").into_response();
    }

    let chunk_dir = state.cache_dir.join("filebox_tmp").join(&payload.upload_id);
    if chunk_dir.exists() {
        if let Err(err) = fs::remove_dir_all(&chunk_dir).await {
            tracing::error!("failed to delete chunk directory for abort: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to clean up upload chunks").into_response();
        }
        tracing::info!("Successfully aborted upload and cleaned up chunks for ID: {}", payload.upload_id);
    }

    (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response()
}

pub fn spawn_filebox_cleanup_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            
            // 1. Clean up expired files from database and disk
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

            // 2. Clean up orphaned/abandoned directories in filebox_tmp older than 24 hours
            let filebox_tmp_dir = state.cache_dir.join("filebox_tmp");
            if let Ok(mut entries) = fs::read_dir(&filebox_tmp_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if let Ok(metadata) = entry.metadata().await {
                        if metadata.is_dir() {
                            if let Ok(modified) = metadata.modified() {
                                if let Ok(elapsed) = modified.elapsed() {
                                    if elapsed.as_secs() > 86400 { // 24 hours
                                        let path = entry.path();
                                        if let Err(err) = fs::remove_dir_all(&path).await {
                                            tracing::warn!("Failed to clean up old temporary chunk dir {}: {}", path.display(), err);
                                        } else {
                                            tracing::info!("Cleaned up abandoned temporary chunk dir: {}", path.display());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });
}
