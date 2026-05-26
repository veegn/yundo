use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use sqlx::Row;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::SystemTime;

use crate::common::AppState;


static UPLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize)]
pub struct InitUploadPayload {
    pub file_name: String,
    pub file_size: i64,
    pub content_type: Option<String>,
    pub replication_factor: Option<i64>,
    pub file_sha256: Option<String>,
}

pub async fn init_upload_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<InitUploadPayload>,
) -> impl IntoResponse {
    if payload.file_name.trim().is_empty() || payload.file_size <= 0 {
        return (StatusCode::BAD_REQUEST, "invalid file metadata").into_response();
    }

    let used_space = crate::cache::get_combined_used_size(&state.cache_dir, &state.db).await;
    if used_space + payload.file_size as u64 > state.max_cache_size {
        return (
            StatusCode::INSUFFICIENT_STORAGE,
            "存储空间不足，无法上传该文件。请先清理空间或提高配额。",
        )
            .into_response();
    }

    let upload_id = format!("upl_{}", generate_unique_id());
    let file_id = format!("file_{}", generate_unique_id());
    let chunk_size = state.node_config.default_chunk_size;
    let total_chunks = ((payload.file_size + chunk_size - 1) / chunk_size).max(1);
    let replication_factor = payload
        .replication_factor
        .unwrap_or(state.node_config.default_replication_factor)
        .max(1);

    let result = sqlx::query(
        "INSERT INTO upload_sessions (
            id, file_id, file_name, file_size, content_type, chunk_size, total_chunks,
            replication_factor, status, expires_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active', datetime('now', '+24 hours'))",
    )
    .bind(&upload_id)
    .bind(&file_id)
    .bind(payload.file_name.trim())
    .bind(payload.file_size)
    .bind(payload.content_type.as_deref())
    .bind(chunk_size)
    .bind(total_chunks)
    .bind(replication_factor)
    .execute(&state.db)
    .await;

    if let Err(err) = result {
        tracing::error!("failed to create upload session: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    if let Some(file_sha256) = payload.file_sha256.as_deref() {
        if is_sha256(file_sha256) {
            let _ = sqlx::query(
                "UPDATE upload_sessions SET updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            )
            .bind(&upload_id)
            .execute(&state.db)
            .await;
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "upload_id": upload_id,
            "file_id": file_id,
            "chunk_size": chunk_size,
            "total_chunks": total_chunks,
            "concurrency_hint": 2,
            "expires_at": null,
        })),
    )
        .into_response()
}

pub async fn get_upload_status_handler(
    State(state): State<Arc<AppState>>,
    Path(upload_id): Path<String>,
) -> impl IntoResponse {
    let session = match load_upload_session(&state, &upload_id).await {
        Ok(Some(session)) => session,
        Ok(None) => return (StatusCode::NOT_FOUND, "upload session not found").into_response(),
        Err(err) => {
            tracing::error!("failed to load upload session: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let rows = match sqlx::query(
        "SELECT chunk_index, status FROM upload_session_chunks WHERE upload_id = ? ORDER BY chunk_index",
    )
    .bind(&upload_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!("failed to load upload chunk status: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let mut uploaded_chunks = Vec::new();
    let mut failed_chunks = Vec::new();
    let mut conflict_chunks = Vec::new();
    let mut seen = vec![false; session.total_chunks as usize];

    for row in rows {
        let index: i64 = row.get("chunk_index");
        let status: String = row.get("status");
        if index >= 0 && index < session.total_chunks {
            seen[index as usize] = true;
            match status.as_str() {
                "uploaded" => uploaded_chunks.push(index),
                "failed" => failed_chunks.push(index),
                "conflict" => conflict_chunks.push(index),
                _ => {}
            }
        }
    }

    let missing_chunks: Vec<i64> = (0..session.total_chunks)
        .filter(|index| !seen[*index as usize])
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": session.status,
            "chunk_size": session.chunk_size,
            "total_chunks": session.total_chunks,
            "uploaded_chunks": uploaded_chunks,
            "missing_chunks": missing_chunks,
            "failed_chunks": failed_chunks,
            "conflict_chunks": conflict_chunks,
        })),
    )
        .into_response()
}

pub async fn put_upload_chunk_handler(
    State(state): State<Arc<AppState>>,
    Path((upload_id, index)): Path<(String, i64)>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let expected_sha256 = match headers
        .get("x-chunk-sha256")
        .and_then(|value| value.to_str().ok())
    {
        Some(value) if is_sha256(value) => value.to_ascii_lowercase(),
        _ => return (StatusCode::BAD_REQUEST, "missing or invalid X-Chunk-Sha256").into_response(),
    };

    let session = match load_upload_session(&state, &upload_id).await {
        Ok(Some(session)) => session,
        Ok(None) => return (StatusCode::NOT_FOUND, "upload session not found").into_response(),
        Err(err) => {
            tracing::error!("failed to load upload session: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    if session.status != "active" {
        return (StatusCode::CONFLICT, "upload session is not active").into_response();
    }
    if index < 0 || index >= session.total_chunks {
        return (StatusCode::BAD_REQUEST, "chunk index out of range").into_response();
    }

    let expected_size = if index == session.total_chunks - 1 {
        session.file_size - (session.chunk_size * index)
    } else {
        session.chunk_size
    };
    if body.len() as i64 != expected_size {
        return (StatusCode::BAD_REQUEST, "invalid chunk size").into_response();
    }

    if let Ok(Some(existing)) = sqlx::query(
        "SELECT sha256, status FROM upload_session_chunks WHERE upload_id = ? AND chunk_index = ?",
    )
    .bind(&upload_id)
    .bind(index)
    .fetch_optional(&state.db)
    .await
    {
        let existing_sha256: Option<String> = existing.get("sha256");
        let existing_status: String = existing.get("status");
        if existing_status == "uploaded" {
            if existing_sha256.as_deref() == Some(expected_sha256.as_str()) {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "success": true,
                        "upload_id": upload_id,
                        "chunk_index": index,
                        "sha256": expected_sha256,
                        "status": "uploaded",
                    })),
                )
                    .into_response();
            }
            let _ = mark_upload_chunk_conflict(&state, &upload_id, index, &expected_sha256).await;
            return (StatusCode::CONFLICT, "chunk sha256 conflict").into_response();
        }
    }

    let used_space = crate::cache::get_combined_used_size(&state.cache_dir, &state.db).await;
    if used_space + body.len() as u64 > state.max_cache_size {
        return (
            StatusCode::INSUFFICIENT_STORAGE,
            "存储空间不足，无法上传分片。请清理空间或提高配额。",
        )
            .into_response();
    }

    let object_key = format!("files/{}/chunks/{}", session.file_id, index);
    let write_result = match state
        .storage_backend
        .put_chunk(&object_key, body, &expected_sha256)
        .await
    {
        Ok(result) => result,
        Err(crate::storage::StorageError::ChecksumMismatch { .. }) => {
            let _ = mark_upload_chunk_failed(&state, &upload_id, index, "checksum mismatch").await;
            return (StatusCode::UNPROCESSABLE_ENTITY, "chunk sha256 mismatch").into_response();
        }
        Err(err) => {
            tracing::error!("failed to write upload chunk: {err}");
            let _ = mark_upload_chunk_failed(&state, &upload_id, index, &err.to_string()).await;
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    };

    let result = sqlx::query(
        "INSERT INTO upload_session_chunks (
            upload_id, chunk_index, size_bytes, sha256, status, node_id, object_key, error, updated_at
        ) VALUES (?, ?, ?, ?, 'uploaded', ?, ?, NULL, CURRENT_TIMESTAMP)
        ON CONFLICT(upload_id, chunk_index) DO UPDATE SET
            size_bytes = excluded.size_bytes,
            sha256 = excluded.sha256,
            status = 'uploaded',
            node_id = excluded.node_id,
            object_key = excluded.object_key,
            error = NULL,
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(&upload_id)
    .bind(index)
    .bind(write_result.size_bytes as i64)
    .bind(&write_result.sha256)
    .bind(&state.node_config.node_id)
    .bind(&object_key)
    .execute(&state.db)
    .await;

    if let Err(err) = result {
        tracing::error!("failed to save uploaded chunk metadata: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "upload_id": upload_id,
            "chunk_index": index,
            "sha256": write_result.sha256,
            "status": "uploaded",
        })),
    )
        .into_response()
}

pub async fn complete_upload_handler(
    State(state): State<Arc<AppState>>,
    Path(upload_id): Path<String>,
) -> impl IntoResponse {
    let session = match load_upload_session(&state, &upload_id).await {
        Ok(Some(session)) => session,
        Ok(None) => return (StatusCode::NOT_FOUND, "upload session not found").into_response(),
        Err(err) => {
            tracing::error!("failed to load upload session: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    if session.status != "active" {
        return (StatusCode::CONFLICT, "upload session is not active").into_response();
    }

    let chunks = match sqlx::query(
        "SELECT chunk_index, size_bytes, sha256, object_key
         FROM upload_session_chunks
         WHERE upload_id = ? AND status = 'uploaded'
         ORDER BY chunk_index",
    )
    .bind(&upload_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!("failed to load uploaded chunks: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    if chunks.len() != session.total_chunks as usize {
        return (StatusCode::BAD_REQUEST, "missing chunks").into_response();
    }

    let mut total_size = 0_i64;
    for (expected_index, row) in chunks.iter().enumerate() {
        let chunk_index: i64 = row.get("chunk_index");
        if chunk_index != expected_index as i64 {
            return (
                StatusCode::BAD_REQUEST,
                format!("missing chunk: {expected_index}"),
            )
                .into_response();
        }
        let size_bytes: i64 = row.get("size_bytes");
        total_size += size_bytes;
    }

    if total_size != session.file_size {
        return (
            StatusCode::BAD_REQUEST,
            "uploaded size does not match session",
        )
            .into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!("failed to begin complete transaction: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let result = async {
        sqlx::query("UPDATE upload_sessions SET status = 'completing', updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(&upload_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO files (
                id, file_name, file_size, content_type, chunk_size, total_chunks, sha256,
                status, replication_factor, expires_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, NULL, 'partial_ready', ?, datetime('now', '+7 days'), CURRENT_TIMESTAMP)",
        )
        .bind(&session.file_id)
        .bind(&session.file_name)
        .bind(session.file_size)
        .bind(session.content_type.as_deref())
        .bind(session.chunk_size)
        .bind(session.total_chunks)
        .bind(session.replication_factor)
        .execute(&mut *tx)
        .await?;

        for row in &chunks {
            let chunk_index: i64 = row.get("chunk_index");
            let size_bytes: i64 = row.get("size_bytes");
            let sha256: String = row.get("sha256");
            let object_key: String = row.get("object_key");
            let chunk_id = format!("chk_{}", generate_unique_id());

            sqlx::query(
                "INSERT INTO file_chunks (id, file_id, chunk_index, size_bytes, sha256, status)
                 VALUES (?, ?, ?, ?, ?, 'ready')",
            )
            .bind(&chunk_id)
            .bind(&session.file_id)
            .bind(chunk_index)
            .bind(size_bytes)
            .bind(&sha256)
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                "INSERT INTO chunk_replicas (
                    chunk_id, node_id, object_key, size_bytes, sha256, status, verified_at
                 ) VALUES (?, ?, ?, ?, ?, 'ready', CURRENT_TIMESTAMP)",
            )
            .bind(&chunk_id)
            .bind(&state.node_config.node_id)
            .bind(&object_key)
            .bind(size_bytes)
            .bind(&sha256)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query("UPDATE upload_sessions SET status = 'completed', updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(&upload_id)
            .execute(&mut *tx)
            .await?;

        Ok::<(), sqlx::Error>(())
    }
    .await;

    if let Err(err) = result {
        let _ = tx.rollback().await;
        tracing::error!("failed to complete upload: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    if let Err(err) = tx.commit().await {
        tracing::error!("failed to commit upload completion: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "file_id": session.file_id,
            "status": "partial_ready",
            "download_url": format!("/api/filebox/download/{}", session.file_id),
        })),
    )
        .into_response()
}

pub async fn abort_upload_handler(
    State(state): State<Arc<AppState>>,
    Path(upload_id): Path<String>,
) -> impl IntoResponse {
    if !is_safe_id(&upload_id) {
        return (StatusCode::BAD_REQUEST, "invalid upload_id").into_response();
    }

    let rows = match sqlx::query("SELECT object_key FROM upload_session_chunks WHERE upload_id = ?")
        .bind(&upload_id)
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!("failed to load upload chunks for abort: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    for row in rows {
        let object_key: Option<String> = row.get("object_key");
        if let Some(object_key) = object_key {
            if let Err(err) = state.storage_backend.delete_chunk(&object_key).await {
                tracing::warn!("failed to delete aborted upload chunk {object_key}: {err}");
            }
        }
    }

    if let Err(err) = sqlx::query("UPDATE upload_sessions SET status = 'aborted', updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(&upload_id)
        .execute(&state.db)
        .await
    {
        tracing::error!("failed to mark upload session aborted: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    let _ = sqlx::query("UPDATE upload_session_chunks SET status = 'deleting', updated_at = CURRENT_TIMESTAMP WHERE upload_id = ?")
        .bind(&upload_id)
        .execute(&state.db)
        .await;

    (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response()
}

#[derive(Clone)]
struct UploadSession {
    file_id: String,
    file_name: String,
    file_size: i64,
    content_type: Option<String>,
    chunk_size: i64,
    total_chunks: i64,
    replication_factor: i64,
    status: String,
}

async fn load_upload_session(
    state: &AppState,
    upload_id: &str,
) -> Result<Option<UploadSession>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT file_id, file_name, file_size, content_type, chunk_size, total_chunks,
                replication_factor, status
         FROM upload_sessions
         WHERE id = ? AND expires_at >= datetime('now')",
    )
    .bind(upload_id)
    .fetch_optional(&state.db)
    .await?;

    Ok(row.map(|row| UploadSession {
        file_id: row.get("file_id"),
        file_name: row.get("file_name"),
        file_size: row.get("file_size"),
        content_type: row.get("content_type"),
        chunk_size: row.get("chunk_size"),
        total_chunks: row.get("total_chunks"),
        replication_factor: row.get("replication_factor"),
        status: row.get("status"),
    }))
}

async fn mark_upload_chunk_failed(
    state: &AppState,
    upload_id: &str,
    index: i64,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO upload_session_chunks (upload_id, chunk_index, status, error, updated_at)
         VALUES (?, ?, 'failed', ?, CURRENT_TIMESTAMP)
         ON CONFLICT(upload_id, chunk_index) DO UPDATE SET
            status = 'failed', error = excluded.error, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(upload_id)
    .bind(index)
    .bind(error)
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn mark_upload_chunk_conflict(
    state: &AppState,
    upload_id: &str,
    index: i64,
    sha256: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO upload_session_chunks (upload_id, chunk_index, sha256, status, error, updated_at)
         VALUES (?, ?, ?, 'conflict', 'sha256 conflict', CURRENT_TIMESTAMP)
         ON CONFLICT(upload_id, chunk_index) DO UPDATE SET
            status = 'conflict', error = 'sha256 conflict', updated_at = CURRENT_TIMESTAMP",
    )
    .bind(upload_id)
    .bind(index)
    .bind(sha256)
    .execute(&state.db)
    .await?;
    Ok(())
}

fn generate_unique_id() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let count = UPLOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
    let raw = format!("{}-{}", now, count);

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let hex = hex::encode(hasher.finalize());
    hex[..16].to_string()
}

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
