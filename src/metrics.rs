use crate::common::AppState;
use axum::{
    body::Body,
    extract::{MatchedPath, State},
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics::{describe_counter, describe_gauge, describe_histogram, Unit};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::{sync::Arc, time::Instant};

pub fn install_metrics_recorder(
) -> Result<PrometheusHandle, metrics_exporter_prometheus::BuildError> {
    let handle = PrometheusBuilder::new().install_recorder()?;

    describe_counter!(
        "yundo_http_requests_total",
        "Total HTTP requests handled by Yundo."
    );
    describe_histogram!(
        "yundo_http_request_duration_seconds",
        Unit::Seconds,
        "HTTP request latency in seconds."
    );
    describe_counter!(
        "yundo_proxy_cache_requests_total",
        "Proxy cache lookups labelled by result."
    );
    describe_counter!(
        "yundo_proxy_upstream_errors_total",
        "Upstream request errors labelled by proxy type."
    );
    describe_counter!(
        "yundo_filebox_upload_chunks_total",
        "Filebox upload chunks accepted."
    );
    describe_histogram!(
        "yundo_filebox_upload_chunk_bytes",
        Unit::Bytes,
        "Accepted filebox upload chunk sizes."
    );
    describe_counter!(
        "yundo_filebox_upload_merges_total",
        "Filebox chunk merge attempts labelled by result."
    );
    describe_histogram!(
        "yundo_filebox_upload_merged_bytes",
        Unit::Bytes,
        "Final merged file sizes for chunked uploads."
    );
    describe_gauge!(
        "yundo_cache_usage_bytes",
        Unit::Bytes,
        "Current estimated cache usage in bytes."
    );

    Ok(handle)
}

pub async fn metrics_handler(State(state): State<Arc<AppState>>) -> Response {
    let Some(handle) = state.metrics_handle.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "metrics recorder is not installed",
        )
            .into_response();
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        handle.render(),
    )
        .into_response()
}

pub async fn track_http_metrics(request: Request<Body>, next: Next) -> Response {
    let method = request.method().as_str().to_string();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let started = Instant::now();

    let response = next.run(request).await;
    let status = response.status().as_u16().to_string();
    let elapsed = started.elapsed().as_secs_f64();

    ::metrics::counter!(
        "yundo_http_requests_total",
        "method" => method.clone(),
        "route" => route.clone(),
        "status" => status.clone(),
    )
    .increment(1);
    ::metrics::histogram!(
        "yundo_http_request_duration_seconds",
        "method" => method,
        "route" => route,
        "status" => status,
    )
    .record(elapsed);

    response
}
