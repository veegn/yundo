use crate::{
    common::{resolve_file_name, AppState},
    constants::{FILEBOX_EXPIRATION_DAYS, MAX_UPLOAD_CHUNK_BYTES},
    errors::AppError,
    filebox_utils::{check_file_size_limit, validate_upload_id},
    storage::{remove_accounted_directory, remove_accounted_file, PendingFile},
};
use axum::{
    body::Body,
    extract::{Multipart, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use futures_util::StreamExt;
use sqlx::Row;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::SystemTime;
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use url::Url;

type ApiResult = Result<Json<serde_json::Value>, AppError>;
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

pub async fn list_filebox_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let used_space =
        crate::cache::get_combined_used_size(&state.cache_dir, &state.db, &state).await;

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

    let files: Vec<serde_json::Value> = files_result
        .into_iter()
        .map(|row| {
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
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total_space": state.max_cache_size,
            "used_space": used_space,
            "auth_required": state.api_key.is_some(),
            "max_file_size": state.max_file_size,
            "files": files,
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub struct RemoteUploadPayload {
    pub url: String,
}

fn check_size(state: &AppState, size: u64) -> Result<(), AppError> {
    check_file_size_limit(state, size).map_err(|_| AppError::FileTooLarge)
}

async fn publish_file(
    state: &AppState,
    pending: &mut PendingFile,
    id: &str,
    name: &str,
) -> Result<(), AppError> {
    pending
        .rename(&state.cache_dir.join("filebox").join(id))
        .await?;
    sqlx::query("INSERT INTO filebox_files (id, file_name, file_size, expires_at) VALUES (?, ?, ?, datetime('now', ?))")
        .bind(id).bind(name).bind(pending.size() as i64).bind(format!("+{FILEBOX_EXPIRATION_DAYS} days"))
        .execute(&state.db).await?;
    Ok(())
}

async fn finish_pending(pending: Option<PendingFile>, result: ApiResult) -> ApiResult {
    if let Some(file) = pending {
        if result.is_ok() {
            file.commit();
        } else {
            file.discard().await;
        }
    }
    result
}

pub async fn remote_upload_filebox_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RemoteUploadPayload>,
) -> ApiResult {
    let parsed_url = Url::parse(&payload.url).map_err(|_| AppError::InvalidUrl)?;
    crate::ssrf::validate_url_host(&parsed_url).map_err(|_| {
        if matches!(parsed_url.scheme(), "http" | "https") {
            AppError::ForbiddenHost
        } else {
            AppError::UnsupportedScheme
        }
    })?;
    let response = state
        .client
        .get(parsed_url.clone())
        .header("User-Agent", "precision-proxy/1.0")
        .send()
        .await
        .map_err(|_| AppError::UpstreamConnectionFailed)?;
    if !response.status().is_success() {
        return Err(AppError::UpstreamError);
    }
    if let Some(length) = response.content_length() {
        check_size(&state, length)?;
    }
    let name = resolve_file_name(&parsed_url, Some(response.url()), response.headers());
    let id = generate_unique_id();
    let operation = Arc::new(state.cache_usage.operations.clone().read_owned().await);
    let mut pending = PendingFile::create(
        state.cache_dir.join("filebox").join(format!("{id}.tmp")),
        state.cache_usage.clone(),
        state.max_cache_size,
        operation.clone(),
    )
    .await?;
    let result = async {
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| AppError::UpstreamError)?;
            check_size(&state, pending.size().checked_add(chunk.len() as u64).ok_or(AppError::FileTooLarge)?)?;
            pending.write_all(&chunk).await?;
        }
        if pending.size() == 0 { return Err(AppError::NoValidFiles); }
        publish_file(&state, &mut pending, &id, &name).await?;
        Ok(Json(serde_json::json!({"success":true,"file":{"id":id,"file_name":name,"file_size":pending.size()}})))
    }.await;
    finish_pending(Some(pending), result).await
}

pub async fn upload_filebox_handler(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> ApiResult {
    let mut uploaded = Vec::new();
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::RequestBodyReadFailed)?
    {
        let Some(name) = field.file_name().map(str::to_string) else {
            continue;
        };
        let id = generate_unique_id();
        let operation = Arc::new(state.cache_usage.operations.clone().read_owned().await);
        let mut pending = PendingFile::create(
            state.cache_dir.join("filebox").join(format!("{id}.tmp")),
            state.cache_usage.clone(),
            state.max_cache_size,
            operation.clone(),
        )
        .await?;
        let result = async {
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|_| AppError::RequestBodyReadFailed)?
            {
                check_size(
                    &state,
                    pending
                        .size()
                        .checked_add(chunk.len() as u64)
                        .ok_or(AppError::FileTooLarge)?,
                )?;
                pending.write_all(&chunk).await?;
            }
            if pending.size() == 0 {
                return Err(AppError::NoValidFiles);
            }
            publish_file(&state, &mut pending, &id, &name).await?;
            Ok(Json(
                serde_json::json!({"id":id,"file_name":name,"file_size":pending.size()}),
            ))
        }
        .await;
        let Json(item) = finish_pending(Some(pending), result).await?;
        uploaded.push(item);
    }
    if uploaded.is_empty() {
        return Err(AppError::NoValidFiles);
    }
    Ok(Json(serde_json::json!({"success":true,"files":uploaded})))
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
        let start = file_size.saturating_sub(suffix_len);
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
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );

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
                return (
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    response_headers,
                    Body::empty(),
                )
                    .into_response();
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
) -> ApiResult {
    validate_upload_id(&id).map_err(|_| AppError::InvalidUploadId)?;
    let _operation = state.cache_usage.operations.read().await;
    let _file_lock = state
        .cache_usage
        .key_lock(&format!("file:{id}"))
        .write_owned()
        .await;
    let exists = sqlx::query("SELECT id FROM filebox_files WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::FileNotFound);
    }
    remove_accounted_file(
        &state.cache_dir.join("filebox").join(&id),
        &state.cache_usage,
    )
    .await?;
    sqlx::query("DELETE FROM filebox_files WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    Ok(Json(serde_json::json!({"success":true})))
}

#[derive(serde::Deserialize)]
pub struct UploadCompletePayload {
    pub upload_id: String,
    pub file_name: String,
    pub total_chunks: usize,
}

async fn small_field(mut field: axum::extract::multipart::Field<'_>) -> Result<String, AppError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|_| AppError::RequestBodyReadFailed)?
    {
        if bytes.len() + chunk.len() > 256 {
            return Err(AppError::InvalidUpload("metadata field too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| AppError::InvalidUpload("metadata must be UTF-8"))
}

pub async fn upload_chunk_handler(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> ApiResult {
    let operation = Arc::new(state.cache_usage.operations.clone().read_owned().await);
    let mut pending: Option<PendingFile> = None;
    let result = async {
        let mut upload_id = None;
        let mut chunk_index = None;
        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|_| AppError::RequestBodyReadFailed)?
        {
            match field.name() {
                Some("upload_id") if upload_id.is_none() => {
                    let id = small_field(field).await?;
                    validate_upload_id(&id).map_err(|_| AppError::InvalidUploadId)?;
                    upload_id = Some(id);
                }
                Some("chunk_index") if chunk_index.is_none() => {
                    chunk_index = Some(
                        small_field(field)
                            .await?
                            .parse::<usize>()
                            .map_err(|_| AppError::InvalidUpload("invalid chunk index"))?,
                    );
                }
                Some("file") if pending.is_none() => {
                    let staged = state
                        .cache_dir
                        .join("filebox_tmp")
                        .join(format!("{}.tmp", generate_unique_id()));
                    pending = Some(
                        PendingFile::create(
                            staged,
                            state.cache_usage.clone(),
                            state.max_cache_size,
                            operation.clone(),
                        )
                        .await?,
                    );
                    let file = pending.as_mut().unwrap();
                    while let Some(chunk) = field
                        .chunk()
                        .await
                        .map_err(|_| AppError::RequestBodyReadFailed)?
                    {
                        let next_size = file
                            .size()
                            .checked_add(chunk.len() as u64)
                            .ok_or(AppError::FileTooLarge)?;
                        if next_size > MAX_UPLOAD_CHUNK_BYTES {
                            return Err(AppError::FileTooLarge);
                        }
                        check_size(&state, next_size)?;
                        file.write_all(&chunk).await?;
                    }
                }
                _ => return Err(AppError::InvalidUpload("unexpected or duplicate field")),
            }
        }
        let id = upload_id.ok_or(AppError::InvalidUpload("missing upload ID"))?;
        let index = chunk_index.ok_or(AppError::InvalidUpload("missing chunk index"))?;
        let file = pending.as_mut().ok_or(AppError::NoValidFiles)?;
        if file.size() == 0 {
            return Err(AppError::NoValidFiles);
        }
        // Only validated identifiers ever become part of a filesystem path.
        let _upload_lock = state
            .cache_usage
            .key_lock(&format!("upload:{id}"))
            .read_owned()
            .await;
        let _chunk_lock = state
            .cache_usage
            .key_lock(&format!("chunk:{id}:{index}"))
            .write_owned()
            .await;
        let directory = state.cache_dir.join("filebox_tmp").join(&id);
        fs::create_dir_all(&directory).await?;
        let destination = directory.join(index.to_string());
        if fs::try_exists(&destination).await? {
            return Err(AppError::UploadConflict);
        }
        file.rename(&destination).await?;
        ::metrics::counter!("yundo_filebox_upload_chunks_total").increment(1);
        ::metrics::histogram!("yundo_filebox_upload_chunk_bytes").record(file.size() as f64);
        Ok(Json(serde_json::json!({"success":true})))
    }
    .await;
    finish_pending(pending, result).await
}

pub async fn upload_complete_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UploadCompletePayload>,
) -> ApiResult {
    let result = complete_upload(state, payload).await;
    ::metrics::counter!("yundo_filebox_upload_merges_total", "result" => if result.is_ok() { "success" } else { "error" }).increment(1);
    result
}

async fn complete_upload(state: Arc<AppState>, payload: UploadCompletePayload) -> ApiResult {
    validate_upload_id(&payload.upload_id).map_err(|_| AppError::InvalidUploadId)?;
    if payload.total_chunks == 0 || payload.file_name.is_empty() {
        return Err(AppError::InvalidUpload("missing file metadata"));
    }
    let operation = Arc::new(state.cache_usage.operations.clone().read_owned().await);
    let _upload_lock = state
        .cache_usage
        .key_lock(&format!("upload:{}", payload.upload_id))
        .write_owned()
        .await;
    let directory = state.cache_dir.join("filebox_tmp").join(&payload.upload_id);
    let mut total = 0_u64;
    for index in 0..payload.total_chunks {
        let size = fs::metadata(directory.join(index.to_string()))
            .await
            .map_err(|_| AppError::MissingChunk(index))?
            .len();
        if size == 0 {
            return Err(AppError::InvalidUpload("empty chunk"));
        }
        total = total.checked_add(size).ok_or(AppError::FileTooLarge)?;
        check_size(&state, total)?;
    }
    // Merging temporarily holds both the source chunks and the assembled file.
    if total > state.max_cache_size.saturating_sub(state.cache_usage.get()) {
        return Err(AppError::StorageFull);
    }
    let id = generate_unique_id();
    let mut pending = PendingFile::create(
        state.cache_dir.join("filebox").join(format!("{id}.tmp")),
        state.cache_usage.clone(),
        state.max_cache_size,
        operation.clone(),
    )
    .await?;
    let result = async {
        let mut buffer = vec![0_u8; 64 * 1024];
        for index in 0..payload.total_chunks {
            let mut chunk = File::open(directory.join(index.to_string())).await?;
            loop {
                let length = chunk.read(&mut buffer).await?;
                if length == 0 { break; }
                pending.write_all(&buffer[..length]).await?;
            }
        }
        publish_file(&state, &mut pending, &id, &payload.file_name).await?;
        Ok(Json(serde_json::json!({"success":true,"files":[{"id":id,"file_name":payload.file_name,"file_size":pending.size()}]})))
    }.await;
    let response = finish_pending(Some(pending), result).await?;
    if let Err(error) = remove_accounted_directory(&directory, &state.cache_usage).await {
        tracing::warn!(%error, "merged chunk cleanup failed; retaining quota");
    }
    ::metrics::histogram!("yundo_filebox_upload_merged_bytes").record(total as f64);
    Ok(response)
}

#[derive(serde::Deserialize)]
pub struct UploadAbortPayload {
    pub upload_id: String,
}

pub async fn upload_abort_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UploadAbortPayload>,
) -> ApiResult {
    validate_upload_id(&payload.upload_id).map_err(|_| AppError::InvalidUploadId)?;
    let _operation = state.cache_usage.operations.read().await;
    let _upload_lock = state
        .cache_usage
        .key_lock(&format!("upload:{}", payload.upload_id))
        .write_owned()
        .await;
    remove_accounted_directory(
        &state.cache_dir.join("filebox_tmp").join(&payload.upload_id),
        &state.cache_usage,
    )
    .await?;
    Ok(Json(serde_json::json!({"success":true})))
}

pub fn spawn_filebox_cleanup_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(crate::constants::FILEBOX_CLEANUP_INTERVAL) => {
                    if let Err(error) = cleanup_filebox(&state).await { tracing::warn!(%error, "FileBox cleanup failed"); }
                }
                _ = state.shutdown_token.cancelled() => break,
            }
        }
    });
}

async fn cleanup_filebox(state: &AppState) -> Result<(), AppError> {
    let Ok(_idle) = state.cache_usage.operations.try_write() else {
        return Ok(());
    };
    let expired = sqlx::query("SELECT id FROM filebox_files WHERE expires_at < datetime('now')")
        .fetch_all(&state.db)
        .await?;
    for row in expired {
        let id: String = row.get("id");
        if let Err(error) = remove_accounted_file(
            &state.cache_dir.join("filebox").join(&id),
            &state.cache_usage,
        )
        .await
        {
            tracing::warn!(%error, %id, "expired file removal failed");
            continue;
        }
        sqlx::query("DELETE FROM filebox_files WHERE id = ?")
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    let mut entries = fs::read_dir(state.cache_dir.join("filebox_tmp")).await?;
    while let Some(entry) = entries.next_entry().await? {
        let mut modified = entry.metadata().await?.modified()?;
        if entry.file_type().await?.is_dir() {
            let mut chunks = fs::read_dir(entry.path()).await?;
            while let Some(chunk) = chunks.next_entry().await? {
                modified = modified.max(chunk.metadata().await?.modified()?);
            }
        }
        if modified.elapsed().unwrap_or_default().as_secs()
            > crate::constants::TEMP_CHUNK_MAX_AGE_SECS
        {
            if entry.file_type().await?.is_dir() {
                remove_accounted_directory(&entry.path(), &state.cache_usage).await?;
            } else {
                remove_accounted_file(&entry.path(), &state.cache_usage).await?;
            }
        }
    }
    Ok(())
}
