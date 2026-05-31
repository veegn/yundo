use crate::{
    common::{ensure_download_filename, AppState, CacheMeta},
    constants::CACHE_EVICTION_INTERVAL,
    history::record_download,
};
use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
};
use std::{path::Path, sync::Arc, time::SystemTime};
use tokio::fs::{self, File};
use tokio_util::io::ReaderStream;

use sqlx::Row;

/// Calculate actual disk usage by traversing directories.
/// This is expensive and should only be called periodically for calibration.
pub async fn calculate_actual_usage(cache_dir: &Path, db: &sqlx::SqlitePool) -> u64 {
    // 1. Get active filebox files size from DB
    let filebox_size = sqlx::query(
        "SELECT COALESCE(SUM(file_size), 0) AS total_size FROM filebox_files WHERE expires_at >= datetime('now')"
    )
    .fetch_one(db)
    .await
    .map(|row| row.get::<i64, _>("total_size") as u64)
    .unwrap_or(0);

    // 2. Calculate directory sizes synchronously in a blocking thread
    let cache_dir_buf = cache_dir.to_path_buf();
    let disk_size = tokio::task::spawn_blocking(move || {
        let mut proxy_cache_size = 0_u64;

        // Calculate proxy cache data files size
        if let Ok(entries) = std::fs::read_dir(&cache_dir_buf) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file()
                        && entry.path().extension().is_some_and(|ext| ext == "data")
                    {
                        proxy_cache_size += metadata.len();
                    }
                }
            }
        }

        // Calculate filebox_tmp size (in-progress chunked uploads)
        let filebox_tmp_dir = cache_dir_buf.join("filebox_tmp");
        let mut tmp_size = 0_u64;
        let mut dirs_to_visit = vec![filebox_tmp_dir];
        while let Some(dir) = dirs_to_visit.pop() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if let Ok(metadata) = entry.metadata() {
                        if metadata.is_dir() {
                            dirs_to_visit.push(entry.path());
                        } else if metadata.is_file() {
                            tmp_size += metadata.len();
                        }
                    }
                }
            }
        }

        proxy_cache_size + tmp_size
    })
    .await
    .unwrap_or(0);

    filebox_size + disk_size
}

/// Get combined used size, using cached value or calculating if needed.
pub async fn get_combined_used_size(
    cache_dir: &Path,
    db: &sqlx::SqlitePool,
    state: &AppState,
) -> u64 {
    // Check if we need to recalibrate
    if state.cache_usage.should_recalibrate().await {
        let actual = calculate_actual_usage(cache_dir, db).await;
        state.cache_usage.set(actual);
        state.cache_usage.mark_calibrated().await;
        ::metrics::gauge!("yundo_cache_usage_bytes").set(actual as f64);
        tracing::debug!("Cache usage recalibrated: {} bytes", actual);
        actual
    } else {
        let usage = state.cache_usage.get();
        ::metrics::gauge!("yundo_cache_usage_bytes").set(usage as f64);
        usage
    }
}

pub fn spawn_cache_eviction_task(state: Arc<AppState>) {
    let shutdown_token = state.shutdown_token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(CACHE_EVICTION_INTERVAL) => {
                    if let Err(err) = enforce_cache_size(&state).await {
                        tracing::error!("cache eviction failed: {err}");
                    }
                }
                _ = shutdown_token.cancelled() => {
                    tracing::info!("Cache eviction task shutting down");
                    break;
                }
            }
        }
    });
}

pub(crate) async fn enforce_cache_size(state: &AppState) -> std::io::Result<()> {
    let cache_dir_buf = state.cache_dir.to_path_buf();

    // 1. Calculate proxy cache files size and gather their modified dates synchronously
    let (mut files, proxy_cache_size) = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        let mut proxy_cache_size = 0_u64;
        if let Ok(entries) = std::fs::read_dir(&cache_dir_buf) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file()
                        && entry.path().extension().is_some_and(|ext| ext == "data")
                    {
                        let size = metadata.len();
                        proxy_cache_size += size;
                        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                        files.push((entry.path(), size, modified));
                    }
                }
            }
        }
        (files, proxy_cache_size)
    })
    .await
    .unwrap_or((Vec::new(), 0));

    // 2. Get active filebox files size from DB
    let filebox_size = sqlx::query(
        "SELECT COALESCE(SUM(file_size), 0) AS total_size FROM filebox_files WHERE expires_at >= datetime('now')"
    )
    .fetch_one(&state.db)
    .await
    .map(|row| row.get::<i64, _>("total_size") as u64)
    .unwrap_or(0);

    let mut total_size = proxy_cache_size + filebox_size;

    // Update the atomic counter
    state.cache_usage.set(total_size);
    ::metrics::gauge!("yundo_cache_usage_bytes").set(total_size as f64);

    if total_size <= state.max_cache_size {
        return Ok(());
    }
    tracing::info!(
        proxy_cache_size,
        filebox_size,
        total_size,
        max_cache_size = state.max_cache_size,
        "cache eviction started"
    );

    // 3. Sort files by modified time and evict oldest proxy cache files
    files.sort_by_key(|(_, _, modified)| *modified);

    for (path, size, _) in &files {
        if total_size <= state.max_cache_size {
            break;
        }
        // Remove the data file and its associated meta file together.
        let meta_path = path.with_extension("meta");
        if fs::remove_file(path).await.is_ok() {
            total_size = total_size.saturating_sub(*size);
            state.cache_usage.sub(*size);
            ::metrics::gauge!("yundo_cache_usage_bytes").set(total_size as f64);
            tracing::info!(
                path = %path.display(),
                size,
                total_size,
                "evicted proxy cache file"
            );
        }
        let _ = fs::remove_file(&meta_path).await;
    }
    tracing::info!(
        total_size,
        max_cache_size = state.max_cache_size,
        "cache eviction finished"
    );

    Ok(())
}

pub(crate) async fn try_serve_from_cache(
    data_path: &Path,
    meta_path: &Path,
    db: sqlx::SqlitePool,
    target_url: String,
    file_name: String,
) -> Option<axum::response::Response> {
    if !(data_path.exists() && meta_path.exists()) {
        ::metrics::counter!("yundo_proxy_cache_requests_total", "result" => "miss").increment(1);
        tracing::info!(target_url = %target_url, "proxy cache miss");
        return None;
    }

    let meta_bytes = match fs::read(meta_path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(
                target_url = %target_url,
                meta_path = %meta_path.display(),
                error = %err,
                "failed to read proxy cache metadata"
            );
            ::metrics::counter!("yundo_proxy_cache_requests_total", "result" => "miss")
                .increment(1);
            return None;
        }
    };
    let cache_meta: CacheMeta = match serde_json::from_slice(&meta_bytes) {
        Ok(meta) => meta,
        Err(err) => {
            tracing::warn!(
                target_url = %target_url,
                meta_path = %meta_path.display(),
                error = %err,
                "failed to parse proxy cache metadata"
            );
            ::metrics::counter!("yundo_proxy_cache_requests_total", "result" => "miss")
                .increment(1);
            return None;
        }
    };
    let file = match File::open(data_path).await {
        Ok(file) => file,
        Err(err) => {
            tracing::warn!(
                target_url = %target_url,
                data_path = %data_path.display(),
                error = %err,
                "failed to open proxy cache data"
            );
            ::metrics::counter!("yundo_proxy_cache_requests_total", "result" => "miss")
                .increment(1);
            return None;
        }
    };
    let file_size = file
        .metadata()
        .await
        .ok()
        .map(|metadata| metadata.len() as i64)
        .unwrap_or(0);

    let mut response_headers = HeaderMap::new();
    for (key, value) in cache_meta.headers {
        let name = HeaderName::try_from(key).ok()?;
        let value = HeaderValue::try_from(value).ok()?;
        response_headers.insert(name, value);
    }
    ensure_download_filename(&mut response_headers, &file_name);

    let status = StatusCode::from_u16(cache_meta.status).unwrap_or(StatusCode::OK);
    tracing::info!(
        file_name = %file_name,
        file_size,
        status = %status,
        "proxy cache hit"
    );
    ::metrics::counter!("yundo_proxy_cache_requests_total", "result" => "hit").increment(1);
    record_download(db, target_url, file_name, file_size).await;

    let body = Body::from_stream(ReaderStream::new(file));
    Some((status, response_headers, body).into_response())
}
