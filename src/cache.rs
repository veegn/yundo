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

/// Count payload bytes on disk, including expired/orphaned files and staged writes.
pub async fn calculate_payload_usage(cache_dir: &Path) -> std::io::Result<u64> {
    let root = cache_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut total = 0_u64;
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_file()
                && (name.ends_with(".data") || name.ends_with(".data.tmp"))
            {
                total = total
                    .checked_add(entry.metadata()?.len())
                    .ok_or_else(|| std::io::Error::other("storage size overflow"))?;
            }
        }
        let mut directories = vec![root.join("filebox"), root.join("filebox_tmp")];
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    directories.push(entry.path());
                } else if kind.is_file() {
                    total = total
                        .checked_add(entry.metadata()?.len())
                        .ok_or_else(|| std::io::Error::other("storage size overflow"))?;
                }
            }
        }
        Ok(total)
    })
    .await
    .map_err(std::io::Error::other)?
}

pub async fn calculate_actual_usage(cache_dir: &Path, _db: &sqlx::SqlitePool) -> u64 {
    calculate_payload_usage(cache_dir)
        .await
        .expect("failed to calculate initial storage usage")
}

pub async fn get_combined_used_size(
    cache_dir: &Path,
    _db: &sqlx::SqlitePool,
    state: &AppState,
) -> u64 {
    if state.cache_usage.should_recalibrate().await {
        // Never overwrite reservations belonging to a live writer.
        if let Ok(_idle) = state.cache_usage.operations.try_write() {
            match calculate_payload_usage(cache_dir).await {
                Ok(actual) => {
                    state.cache_usage.set(actual);
                    state.cache_usage.mark_calibrated().await;
                }
                Err(error) => tracing::warn!(%error, "storage calibration failed"),
            }
        }
    }
    let usage = state.cache_usage.get();
    ::metrics::gauge!("yundo_cache_usage_bytes").set(usage as f64);
    usage
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
                _ = shutdown_token.cancelled() => break,
            }
        }
    });
}

pub(crate) async fn enforce_cache_size(state: &AppState) -> std::io::Result<()> {
    let Ok(_idle) = state.cache_usage.operations.try_write() else {
        return Ok(());
    };
    let mut total_size = calculate_payload_usage(&state.cache_dir).await?;
    state.cache_usage.set(total_size);
    state.cache_usage.mark_calibrated().await;
    let mut files = Vec::new();
    let mut entries = fs::read_dir(&state.cache_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_file()
            && entry.path().extension().is_some_and(|ext| ext == "data")
        {
            let metadata = entry.metadata().await?;
            files.push((
                entry.path(),
                metadata.len(),
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ));
        }
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total_size <= state.max_cache_size {
            break;
        }
        if fs::remove_file(&path).await.is_ok() {
            total_size = total_size.saturating_sub(size);
            state.cache_usage.sub(size);
            let _ = fs::remove_file(path.with_extension("meta")).await;
        }
    }
    ::metrics::gauge!("yundo_cache_usage_bytes").set(total_size as f64);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{initialize_cache_dir, initialize_database, CacheUsageTracker};

    #[tokio::test]
    async fn eviction_counts_chunks_and_orphans_without_overwriting_live_reservations() {
        let directory = tempfile::TempDir::new().unwrap();
        initialize_cache_dir(directory.path()).await;
        let state = AppState {
            client: reqwest::Client::new(),
            cache_dir: directory.path().to_path_buf(),
            max_cache_size: 12,
            max_file_size: 0,
            filebox_size: 12,
            db: initialize_database(directory.path()).await,
            frontend_dist: directory.path().join("missing"),
            base_path: "/".into(),
            cache_usage: Arc::new(CacheUsageTracker::new()),
            api_key: None,
            metrics_handle: None,
            shutdown_token: tokio_util::sync::CancellationToken::new(),
        };
        fs::write(directory.path().join("old.data"), b"abcdef")
            .await
            .unwrap();
        fs::create_dir(directory.path().join("filebox_tmp/upload"))
            .await
            .unwrap();
        fs::write(directory.path().join("filebox_tmp/upload/0"), b"12345678")
            .await
            .unwrap();
        fs::write(directory.path().join("filebox/orphan"), b"abcd")
            .await
            .unwrap();
        state.cache_usage.set(100);
        let active = state.cache_usage.operations.read().await;
        enforce_cache_size(&state).await.unwrap();
        assert_eq!(state.cache_usage.get(), 100);
        drop(active);
        enforce_cache_size(&state).await.unwrap();
        assert_eq!(state.cache_usage.get(), 12);
        assert_eq!(calculate_payload_usage(directory.path()).await.unwrap(), 12);
        assert!(!directory.path().join("old.data").exists());
    }
}
