use crate::{
    cache::try_serve_from_cache,
    common::{
        build_content_disposition, ensure_download_filename, extract_filename_from_url,
        is_forbidden_hostname, resolve_file_name, AppState, ProxyQuery, ALLOWED_HEADERS,
    },
    history::record_download,
    storage::{remove_accounted_file, PendingFile},
};
use axum::{
    body::Body,
    extract::{Query, State},
    http::HeaderMap,
    response::IntoResponse,
};
use bytes::Bytes;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use tokio::{fs, sync::mpsc};
use tokio_stream::wrappers::ReceiverStream;
use url::Url;

pub(crate) async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProxyQuery>,
    req_headers: HeaderMap,
) -> impl IntoResponse {
    let target_url = query.url;
    tracing::info!(target_url = %target_url, "proxy request received");

    let parsed_url = match Url::parse(&target_url) {
        Ok(url) => url,
        Err(err) => {
            tracing::warn!(target_url = %target_url, error = %err, "proxy request rejected: invalid URL");
            return (axum::http::StatusCode::BAD_REQUEST, "invalid URL format").into_response();
        }
    };

    if !matches!(parsed_url.scheme(), "http" | "https") {
        tracing::warn!(
            target_url = %target_url,
            scheme = %parsed_url.scheme(),
            "proxy request rejected: unsupported scheme"
        );
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "only HTTP and HTTPS URLs are supported",
        )
            .into_response();
    }

    let host = parsed_url
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if is_forbidden_hostname(&host) {
        tracing::warn!(target_url = %target_url, host = %host, "proxy request rejected: forbidden host");
        return (
            axum::http::StatusCode::FORBIDDEN,
            "access to local or private networks is forbidden",
        )
            .into_response();
    }

    let initial_file_name =
        extract_filename_from_url(&parsed_url).unwrap_or_else(|| "download.bin".to_string());

    let range_value = req_headers
        .get("range")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let is_range_request = !range_value.is_empty();
    tracing::info!(
        target_url = %target_url,
        range = %range_value,
        is_range_request,
        "proxy request accepted"
    );

    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(target_url.as_bytes());
        hasher.update(b"|");
        hasher.update(range_value.as_bytes());
        hex::encode(hasher.finalize())
    };

    let data_path = state.cache_dir.join(format!("{hash}.data"));
    let meta_path = state.cache_dir.join(format!("{hash}.meta"));

    let mut operation = Some(state.cache_usage.operations.clone().read_owned().await);
    let cache_guard = if is_range_request {
        operation.take();
        None
    } else {
        Some(
            state
                .cache_usage
                .key_lock(&format!("proxy:{hash}"))
                .write_owned()
                .await,
        )
    };

    if !is_range_request {
        tracing::info!(target_url = %target_url, cache_key = %hash, "checking proxy cache");
        if let Some(response) = try_serve_from_cache(
            &data_path,
            &meta_path,
            state.db.clone(),
            target_url.clone(),
            initial_file_name.clone(),
        )
        .await
        {
            return response;
        }
    }

    // A miss can leave an old data file behind when metadata is unreadable.
    let cache_writable = if !is_range_request {
        let removed = remove_accounted_file(&data_path, &state.cache_usage)
            .await
            .is_ok();
        if removed {
            let _ = fs::remove_file(&meta_path).await;
        }
        removed
    } else {
        false
    };

    let mut upstream_request = state.client.get(&target_url);
    if let Some(range) = req_headers.get("range") {
        upstream_request = upstream_request.header("Range", range);
    }
    upstream_request = upstream_request.header("User-Agent", "precision-proxy/1.0");

    let upstream_response = match upstream_request.send().await {
        Ok(response) => response,
        Err(err) => {
            tracing::error!(target_url = %target_url, error = %err, "proxy upstream request failed");
            ::metrics::counter!("yundo_proxy_upstream_errors_total", "proxy" => "download")
                .increment(1);
            return (
                axum::http::StatusCode::BAD_GATEWAY,
                "failed to reach target server",
            )
                .into_response();
        }
    };
    let final_url = upstream_response.url().clone();

    let status = upstream_response.status();
    let mut response_headers = HeaderMap::new();
    let mut meta_headers = HashMap::new();
    let mut file_size = 0_i64;

    for (name, value) in upstream_response.headers() {
        let header_name = name.as_str().to_ascii_lowercase();
        if header_name == "content-length" {
            file_size = value
                .to_str()
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0);
        }
        if header_name == "content-range" && file_size == 0 {
            file_size = value
                .to_str()
                .ok()
                .and_then(extract_total_size_from_content_range)
                .unwrap_or(0);
        }

        if ALLOWED_HEADERS.contains(&header_name.as_str()) {
            response_headers.insert(name.clone(), value.clone());
            if let Ok(value_text) = value.to_str() {
                meta_headers.insert(header_name, value_text.to_string());
            }
        }
    }

    let file_name = resolve_file_name(&parsed_url, Some(&final_url), &response_headers);
    tracing::info!(
        target_url = %target_url,
        final_url = %final_url,
        status = %status,
        file_name = %file_name,
        file_size,
        is_range_request,
        "proxy upstream response received"
    );

    ensure_download_filename(&mut response_headers, &file_name);
    if !meta_headers.contains_key("content-disposition") {
        meta_headers.insert(
            "content-disposition".to_string(),
            build_content_disposition(&file_name),
        );
    }

    let cache_meta = crate::common::CacheMeta {
        headers: meta_headers,
        status: status.as_u16(),
    };
    let should_cache = status == axum::http::StatusCode::OK && cache_writable;
    let expected_length = upstream_response.content_length();
    if status.is_success() {
        record_download(
            state.db.clone(),
            target_url.clone(),
            file_name.clone(),
            file_size,
        )
        .await;
    }
    let mut stream = upstream_response.bytes_stream();
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(16);

    let tmp_data_path = state.cache_dir.join(format!("{hash}.data.tmp"));
    let tmp_meta_path = state.cache_dir.join(format!("{hash}.meta.tmp"));
    let cache_key = hash.clone();
    let stream_target_url = target_url.clone();

    tokio::spawn(async move {
        let _cache_guard = cache_guard;
        let mut pending = if should_cache {
            PendingFile::create(
                tmp_data_path.clone(),
                state.cache_usage.clone(),
                state.max_cache_size,
                Arc::new(operation.take().unwrap()),
            )
            .await
            .ok()
        } else {
            None
        };
        drop(operation);
        let mut success = true;
        let mut bytes_streamed = 0_u64;
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    bytes_streamed += chunk.len() as u64;
                    if let Some(file) = pending.as_mut() {
                        if let Err(error) = file.write_all(&chunk).await {
                            tracing::debug!(%error, cache_key = %cache_key, "cache write skipped; continuing download");
                            pending.take().unwrap().discard().await;
                        }
                    }
                    if tx.send(Ok(chunk)).await.is_err() {
                        success = false;
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(target_url = %stream_target_url, %error, "proxy upstream stream failed");
                    let _ = tx.send(Err(std::io::Error::other(error.to_string()))).await;
                    ::metrics::counter!("yundo_proxy_upstream_errors_total", "proxy" => "download_stream").increment(1);
                    success = false;
                    break;
                }
            }
        }
        success &= expected_length.is_none_or(|length| length == bytes_streamed);
        let mut cached = false;
        if let Some(mut file) = pending {
            let finalized = if success {
                if let Ok(meta_bytes) = serde_json::to_vec(&cache_meta) {
                    fs::write(&tmp_meta_path, meta_bytes).await.is_ok()
                        && file.rename(&data_path).await.is_ok()
                        && fs::rename(&tmp_meta_path, &meta_path).await.is_ok()
                } else {
                    false
                }
            } else {
                false
            };
            if finalized {
                file.commit();
                cached = true;
                tracing::info!(target_url = %stream_target_url, cache_key = %cache_key, bytes_streamed, "proxy response cached");
            } else {
                file.discard().await;
                let _ = fs::remove_file(&tmp_meta_path).await;
            }
        }
        tracing::info!(target_url = %stream_target_url, cache_key = %cache_key, bytes_streamed, cached, success, "proxy stream finished");
    });
    (
        status,
        response_headers,
        Body::from_stream(ReceiverStream::new(rx)),
    )
        .into_response()
}

pub(crate) async fn proxy_head_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProxyQuery>,
    req_headers: HeaderMap,
) -> impl IntoResponse {
    let target_url = query.url;
    tracing::info!(target_url = %target_url, "proxy HEAD request received");

    let parsed_url = match Url::parse(&target_url) {
        Ok(url) => url,
        Err(err) => {
            tracing::warn!(target_url = %target_url, error = %err, "proxy HEAD request rejected: invalid URL");
            return (axum::http::StatusCode::BAD_REQUEST, "invalid URL format").into_response();
        }
    };

    if !matches!(parsed_url.scheme(), "http" | "https") {
        tracing::warn!(
            target_url = %target_url,
            scheme = %parsed_url.scheme(),
            "proxy HEAD request rejected: unsupported scheme"
        );
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "only HTTP and HTTPS URLs are supported",
        )
            .into_response();
    }

    let host = parsed_url
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if is_forbidden_hostname(&host) {
        tracing::warn!(target_url = %target_url, host = %host, "proxy HEAD request rejected: forbidden host");
        return (
            axum::http::StatusCode::FORBIDDEN,
            "access to local or private networks is forbidden",
        )
            .into_response();
    }

    let mut upstream_request = state.client.head(&target_url);
    if let Some(range) = req_headers.get("range") {
        upstream_request = upstream_request.header("Range", range);
    }
    upstream_request = upstream_request.header("User-Agent", "precision-proxy/1.0");

    let upstream_response = match upstream_request.send().await {
        Ok(response) => response,
        Err(err) => {
            tracing::error!(target_url = %target_url, error = %err, "proxy HEAD upstream request failed");
            return (
                axum::http::StatusCode::BAD_GATEWAY,
                "failed to reach target server",
            )
                .into_response();
        }
    };

    let final_url = upstream_response.url().clone();
    let status = upstream_response.status();
    let mut response_headers = HeaderMap::new();

    for (name, value) in upstream_response.headers() {
        let header_name = name.as_str().to_ascii_lowercase();
        if ALLOWED_HEADERS.contains(&header_name.as_str()) {
            response_headers.insert(name.clone(), value.clone());
        }
    }

    let file_name = resolve_file_name(&parsed_url, Some(&final_url), &response_headers);
    ensure_download_filename(&mut response_headers, &file_name);
    tracing::info!(
        target_url = %target_url,
        final_url = %final_url,
        status = %status,
        file_name = %file_name,
        "proxy HEAD response received"
    );

    (status, response_headers).into_response()
}

fn extract_total_size_from_content_range(value: &str) -> Option<i64> {
    value
        .rsplit_once('/')
        .and_then(|(_, total)| total.parse::<i64>().ok())
}
