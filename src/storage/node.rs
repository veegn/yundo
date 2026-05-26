use crate::common::AppState;
use axum::{
    body::{Body, Bytes},
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use tokio_util::io::ReaderStream;

#[derive(Deserialize)]
pub struct ObjectKeyQuery {
    pub object_key: String,
}

/// PUT /internal/chunks?object_key=<key>
/// Write a chunk to local storage with sha256 verification.
pub async fn put_chunk_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ObjectKeyQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let expected_sha256 = match headers
        .get("x-chunk-sha256")
        .and_then(|v| v.to_str().ok())
    {
        Some(v) if v.len() == 64 => v.to_ascii_lowercase(),
        _ => return (StatusCode::BAD_REQUEST, "missing or invalid X-Chunk-Sha256").into_response(),
    };

    match state
        .storage_backend
        .put_chunk(&query.object_key, body, &expected_sha256)
        .await
    {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "object_key": query.object_key,
                "size_bytes": result.size_bytes,
                "sha256": result.sha256,
            })),
        )
            .into_response(),
        Err(crate::storage::StorageError::InvalidObjectKey) => {
            (StatusCode::BAD_REQUEST, "invalid object key").into_response()
        }
        Err(crate::storage::StorageError::ChecksumMismatch { expected, actual }) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("checksum mismatch: expected {expected}, got {actual}"),
        )
            .into_response(),
        Err(crate::storage::StorageError::Conflict(msg)) => {
            (StatusCode::CONFLICT, msg).into_response()
        }
        Err(err) => {
            tracing::error!("internal put_chunk failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response()
        }
    }
}

/// GET /internal/chunks?object_key=<key>
/// Stream a chunk from local storage.
pub async fn get_chunk_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ObjectKeyQuery>,
) -> impl IntoResponse {
    match state.storage_backend.open_chunk(&query.object_key).await {
        Ok(file) => {
            let stream = ReaderStream::new(file);
            let body = Body::from_stream(stream);
            (
                StatusCode::OK,
                [("content-type", "application/octet-stream")],
                body,
            )
                .into_response()
        }
        Err(crate::storage::StorageError::InvalidObjectKey) => {
            (StatusCode::BAD_REQUEST, "invalid object key").into_response()
        }
        Err(crate::storage::StorageError::NotFound) => {
            (StatusCode::NOT_FOUND, "object not found").into_response()
        }
        Err(err) => {
            tracing::error!("internal get_chunk failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response()
        }
    }
}

/// DELETE /internal/chunks?object_key=<key>
/// Delete a chunk from local storage (idempotent).
pub async fn delete_chunk_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ObjectKeyQuery>,
) -> impl IntoResponse {
    match state.storage_backend.delete_chunk(&query.object_key).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response(),
        Err(crate::storage::StorageError::InvalidObjectKey) => {
            (StatusCode::BAD_REQUEST, "invalid object key").into_response()
        }
        Err(err) => {
            tracing::error!("internal delete_chunk failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    pub object_key: String,
    pub expected_sha256: String,
}

/// POST /internal/chunks/verify
/// Verify a chunk's integrity against expected sha256.
pub async fn verify_chunk_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<VerifyRequest>,
) -> impl IntoResponse {
    match state
        .storage_backend
        .verify_chunk(&payload.object_key, &payload.expected_sha256)
        .await
    {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "exists": result.exists,
                "valid": result.valid,
                "size_bytes": result.size_bytes,
                "sha256": result.sha256,
            })),
        )
            .into_response(),
        Err(crate::storage::StorageError::InvalidObjectKey) => {
            (StatusCode::BAD_REQUEST, "invalid object key").into_response()
        }
        Err(err) => {
            tracing::error!("internal verify_chunk failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response()
        }
    }
}

/// GET /internal/healthz
/// Storage node health check.
pub async fn healthz_handler() -> &'static str {
    "ok"
}
