use crate::{
    common::{ensure_download_filename, AppState, CacheMeta},
    history::record_download,
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

pub fn spawn_cache_eviction_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            if let Err(err) = enforce_cache_size(&state.cache_dir, state.max_cache_size).await {
                tracing::error!("cache eviction failed: {err}");
            }
        }
    });
}

pub(crate) async fn enforce_cache_size(cache_dir: &Path, max_size: u64) -> std::io::Result<()> {
    let mut total_size = 0;
    let mut files = Vec::new();

    let mut entries = fs::read_dir(cache_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.metadata().await?;
        if metadata.is_file() {
            let size = metadata.len();
            total_size += size;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            files.push((entry.path(), size, modified));
        }
    }

    if total_size <= max_size {
        return Ok(());
    }

    // Only evict cache data files — never touch proxy.db or other non-cache files.
    files.retain(|(path, _, _)| {
        path.extension().map_or(false, |ext| ext == "data")
    });
    files.sort_by_key(|(_, _, modified)| *modified);

    for (path, size, _) in &files {
        if total_size <= max_size {
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
    db: sqlx::SqlitePool,
    target_url: String,
    file_name: String,
) -> Option<axum::response::Response> {
    if !(data_path.exists() && meta_path.exists()) {
        return None;
    }

    let meta_bytes = fs::read(meta_path).await.ok()?;
    let cache_meta: CacheMeta = serde_json::from_slice(&meta_bytes).ok()?;
    let file = File::open(data_path).await.ok()?;
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

    record_download(db, target_url, file_name, file_size).await;

    let status = StatusCode::from_u16(cache_meta.status).unwrap_or(StatusCode::OK);
    let body = Body::from_stream(ReaderStream::new(file));
    Some((status, response_headers, body).into_response())
}
