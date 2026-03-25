use crate::{
    cache::try_serve_from_cache,
    common::{
        build_content_disposition, ensure_download_filename, extract_filename_from_url,
        is_forbidden_host, resolve_file_name, AppState, ProxyQuery, ALLOWED_HEADERS,
    },
    history::record_download,
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
use tokio::{fs::{self, File}, io::AsyncWriteExt, sync::mpsc};
use tokio_stream::wrappers::ReceiverStream;
use url::Url;

pub(crate) async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProxyQuery>,
    req_headers: HeaderMap,
) -> impl IntoResponse {
    let target_url = query.url;
    let parsed_url = match Url::parse(&target_url) {
        Ok(url) => url,
        Err(_) => return (axum::http::StatusCode::BAD_REQUEST, "invalid URL format").into_response(),
    };

    if !matches!(parsed_url.scheme(), "http" | "https") {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "only HTTP and HTTPS URLs are supported",
        )
            .into_response();
    }

    let host = parsed_url.host_str().unwrap_or_default().to_ascii_lowercase();
    if is_forbidden_host(&host) {
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

    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(target_url.as_bytes());
        hasher.update(b"|");
        hasher.update(range_value.as_bytes());
        hex::encode(hasher.finalize())
    };

    let data_path = state.cache_dir.join(format!("{hash}.data"));
    let meta_path = state.cache_dir.join(format!("{hash}.meta"));

    if !is_range_request {
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

    let mut upstream_request = state.client.get(&target_url);
    if let Some(range) = req_headers.get("range") {
        upstream_request = upstream_request.header("Range", range);
    }
    upstream_request = upstream_request.header("User-Agent", "precision-proxy/1.0");

    let upstream_response = match upstream_request.send().await {
        Ok(response) => response,
        Err(err) => {
            tracing::error!("proxy request failed for {target_url}: {err}");
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

    ensure_download_filename(&mut response_headers, &file_name);
    if !meta_headers.contains_key("content-disposition") {
        meta_headers.insert(
            "content-disposition".to_string(),
            build_content_disposition(&file_name),
        );
    }

    if status.is_success() {
        record_download(state.db.clone(), target_url.clone(), file_name.clone(), file_size).await;
    }

    let cache_meta = crate::common::CacheMeta {
        headers: meta_headers,
        status: status.as_u16(),
    };
    let should_cache = status.is_success() && !is_range_request;
    let mut stream = upstream_response.bytes_stream();
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(16);

    let tmp_data_path = state.cache_dir.join(format!("{hash}.data.tmp"));
    let tmp_meta_path = state.cache_dir.join(format!("{hash}.meta.tmp"));

    tokio::spawn(async move {
        let mut temp_file = if should_cache {
            File::create(&tmp_data_path).await.ok()
        } else {
            None
        };
        let mut success = true;

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    if let Some(file) = temp_file.as_mut() {
                        if let Err(err) = file.write_all(&chunk).await {
                            tracing::error!("failed to write cache chunk: {err}");
                            success = false;
                            break;
                        }
                    }

                    if tx.send(Ok(chunk)).await.is_err() {
                        success = false;
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx
                        .send(Err(std::io::Error::other(err.to_string())))
                        .await;
                    success = false;
                    break;
                }
            }
        }

        if let Some(file) = temp_file.as_mut() {
            let _ = file.flush().await;
        }

        if should_cache && success {
            if let Ok(meta_bytes) = serde_json::to_vec(&cache_meta) {
                if fs::write(&tmp_meta_path, meta_bytes).await.is_ok()
                    && fs::rename(&tmp_data_path, &data_path).await.is_ok()
                    && fs::rename(&tmp_meta_path, &meta_path).await.is_ok()
                {
                    return;
                }
            }
        }

        let _ = fs::remove_file(&tmp_data_path).await;
        let _ = fs::remove_file(&tmp_meta_path).await;
    });

    (status, response_headers, Body::from_stream(ReceiverStream::new(rx))).into_response()
}

pub(crate) async fn proxy_head_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProxyQuery>,
    req_headers: HeaderMap,
) -> impl IntoResponse {
    let target_url = query.url;
    let parsed_url = match Url::parse(&target_url) {
        Ok(url) => url,
        Err(_) => return (axum::http::StatusCode::BAD_REQUEST, "invalid URL format").into_response(),
    };

    if !matches!(parsed_url.scheme(), "http" | "https") {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "only HTTP and HTTPS URLs are supported",
        )
            .into_response();
    }

    let host = parsed_url.host_str().unwrap_or_default().to_ascii_lowercase();
    if is_forbidden_host(&host) {
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
            tracing::error!("proxy HEAD request failed for {target_url}: {err}");
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

    (status, response_headers).into_response()
}

fn extract_total_size_from_content_range(value: &str) -> Option<i64> {
    value
        .rsplit_once('/')
        .and_then(|(_, total)| total.parse::<i64>().ok())
}
