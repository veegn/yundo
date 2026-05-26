use crate::{
    common::{ensure_download_filename, AppState, CacheMeta},
};
use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
};
use std::{path::Path, sync::Arc, time::SystemTime};
use tokio::{
    fs::{self, File},
};
use tokio_util::io::ReaderStream;

use sqlx::Row;

pub async fn get_combined_used_size(cache_dir: &Path, db: &sqlx::SqlitePool) -> u64 {
    // 1. Get active filebox files size from DB
    let filebox_size = sqlx::query("SELECT COALESCE(SUM(file_size), 0) AS total_size FROM filebox_files WHERE expires_at >= datetime('now')")
        .fetch_one(db)
        .await
        .map(|row| row.get::<i64, _>("total_size") as u64)
        .unwrap_or(0);

    // 2. Calculate directory sizes synchronously in a blocking thread to avoid async task-spawning latency
    let cache_dir_buf = cache_dir.to_path_buf();
    let disk_size = tokio::task::spawn_blocking(move || {
        let mut proxy_cache_size = 0_u64;
        
        // Calculate proxy cache data files size
        if let Ok(entries) = std::fs::read_dir(&cache_dir_buf) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() && entry.path().extension().is_some_and(|ext| ext == "data") {
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

pub fn spawn_cache_eviction_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            if let Err(err) = enforce_cache_size(&state).await {
                tracing::error!("cache eviction failed: {err}");
            }
        }
    });
}

pub(crate) async fn enforce_cache_size(state: &AppState) -> std::io::Result<()> {
    let cache_dir_buf = state.cache_dir.to_path_buf();
    
    // 1. Calculate proxy cache files size and gather their modified dates synchronously inside spawn_blocking
    let (mut files, proxy_cache_size) = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        let mut proxy_cache_size = 0_u64;
        if let Ok(entries) = std::fs::read_dir(&cache_dir_buf) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() && entry.path().extension().is_some_and(|ext| ext == "data") {
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
    let filebox_size = sqlx::query("SELECT COALESCE(SUM(file_size), 0) AS total_size FROM filebox_files WHERE expires_at >= datetime('now')")
        .fetch_one(&state.db)
        .await
        .map(|row| row.get::<i64, _>("total_size") as u64)
        .unwrap_or(0);

    let mut total_size = proxy_cache_size + filebox_size;

    if total_size <= state.max_cache_size {
        return Ok(());
    }

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
        }
        let _ = fs::remove_file(&meta_path).await;
    }

    Ok(())
}

pub(crate) async fn try_serve_from_cache(
    data_path: &Path,
    meta_path: &Path,
    _db: sqlx::SqlitePool,
    _target_url: String,
    file_name: String,
) -> Option<axum::response::Response> {
    if !(data_path.exists() && meta_path.exists()) {
        return None;
    }

    let meta_bytes = fs::read(meta_path).await.ok()?;
    let cache_meta: CacheMeta = serde_json::from_slice(&meta_bytes).ok()?;
    let file = File::open(data_path).await.ok()?;

    let mut response_headers = HeaderMap::new();
    for (key, value) in cache_meta.headers {
        let name = HeaderName::try_from(key).ok()?;
        let value = HeaderValue::try_from(value).ok()?;
        response_headers.insert(name, value);
    }
    ensure_download_filename(&mut response_headers, &file_name);

    let status = StatusCode::from_u16(cache_meta.status).unwrap_or(StatusCode::OK);
    let body = Body::from_stream(ReaderStream::new(file));
    Some((status, response_headers, body).into_response())
}
