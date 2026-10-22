use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Redirect},
    routing::get,
    Router,
};
use precision_proxy::{
    app::build_router,
    cache::calculate_payload_usage,
    ssrf::safe_client_builder,
    state::{initialize_cache_dir, initialize_database, AppState, CacheUsageTracker},
};
use reqwest::Client;
use serde_json::{json, Value};
use std::{
    net::SocketAddr,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::{fs, net::TcpListener};

async fn state(directory: &Path, quota: u64, file_limit: u64) -> AppState {
    initialize_cache_dir(directory).await;
    AppState {
        client: safe_client_builder().build().unwrap(),
        cache_dir: directory.to_path_buf(),
        max_cache_size: quota,
        max_file_size: file_limit,
        filebox_size: quota,
        db: initialize_database(directory).await,
        frontend_dist: directory.join("missing-frontend"),
        base_path: "/".into(),
        cache_usage: Arc::new(CacheUsageTracker::new()),
        api_key: None,
        metrics_handle: None,
        shutdown_token: tokio_util::sync::CancellationToken::new(),
    }
}

async fn router_server(router: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    address
}

async fn server(state: Arc<AppState>) -> SocketAddr {
    router_server(build_router(state.clone(), state.frontend_dist.clone())).await
}

fn multipart(fields: &[(&str, &str)], file: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--test-boundary\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(b"--test-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(file);
    body.extend_from_slice(b"\r\n--test-boundary--\r\n");
    body
}

async fn chunk(address: SocketAddr, id: &str, index: &str, bytes: &[u8]) -> reqwest::Response {
    Client::new()
        .post(format!("http://{address}/api/filebox/upload-chunk"))
        .header(
            "Content-Type",
            "multipart/form-data; boundary=test-boundary",
        )
        .body(multipart(
            &[("upload_id", id), ("chunk_index", index)],
            bytes,
        ))
        .send()
        .await
        .unwrap()
}

async fn post(address: SocketAddr, endpoint: &str, payload: Value) -> reqwest::Response {
    Client::new()
        .post(format!("http://{address}/api/filebox/{endpoint}"))
        .header("Content-Type", "application/json")
        .body(payload.to_string())
        .send()
        .await
        .unwrap()
}

async fn idle(state: &AppState) {
    drop(
        tokio::time::timeout(Duration::from_secs(3), state.cache_usage.operations.write())
            .await
            .unwrap(),
    );
}

#[tokio::test]
async fn chunk_rejects_traversal_and_invalid_metadata_without_leaking_space() {
    let directory = TempDir::new().unwrap();
    let state = Arc::new(state(directory.path(), 128, 0).await);
    let address = server(state.clone()).await;
    for id in ["../escape", "..\\escape", "/absolute", ""] {
        assert_eq!(
            chunk(address, id, "0", b"test").await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        chunk(address, "valid", "bad-index", b"test").await.status(),
        StatusCode::BAD_REQUEST
    );
    idle(&state).await;
    assert!(!directory.path().join("escape").exists());
    assert_eq!(calculate_payload_usage(directory.path()).await.unwrap(), 0);
    assert_eq!(state.cache_usage.get(), 0);
}

#[tokio::test]
async fn chunks_merge_limit_abort_delete_and_duplicates_keep_exact_quota() {
    let directory = TempDir::new().unwrap();
    let state = Arc::new(state(directory.path(), 64, 8).await);
    let address = server(state.clone()).await;
    for index in 0..3 {
        assert_eq!(
            chunk(address, "large", &index.to_string(), b"abcdef")
                .await
                .status(),
            StatusCode::OK
        );
    }
    let response = post(
        address,
        "upload-complete",
        json!({"upload_id":"large","file_name":"test.bin","total_chunks":3}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(state.cache_usage.get(), 18);
    assert_eq!(
        post(address, "upload-abort", json!({"upload_id":"large"}))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(state.cache_usage.get(), 0);
    assert_eq!(
        chunk(address, "small", "0", b"abcdef").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        chunk(address, "small", "0", b"changed").await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(state.cache_usage.get(), 6);
    assert_eq!(
        fs::read(directory.path().join("filebox_tmp/small/0"))
            .await
            .unwrap(),
        b"abcdef"
    );
    let response = post(
        address,
        "upload-complete",
        json!({"upload_id":"small","file_name":"test.bin","total_chunks":1}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    let id = body["files"][0]["id"].as_str().unwrap();
    assert_eq!(state.cache_usage.get(), 6);
    assert_eq!(calculate_payload_usage(directory.path()).await.unwrap(), 6);
    let response = Client::new()
        .delete(format!("http://{address}/api/filebox/delete/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(state.cache_usage.get(), 0);
    assert_eq!(calculate_payload_usage(directory.path()).await.unwrap(), 0);
}

#[tokio::test]
async fn merge_accounts_for_temporary_output_and_preserves_chunks_on_failure() {
    let directory = TempDir::new().unwrap();
    let state = Arc::new(state(directory.path(), 10, 0).await);
    let address = server(state.clone()).await;
    assert_eq!(
        chunk(address, "upload", "0", b"abcdef").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        post(
            address,
            "upload-complete",
            json!({"upload_id":"upload","file_name":"file","total_chunks":1})
        )
        .await
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(state.cache_usage.get(), 6);
    assert!(directory.path().join("filebox_tmp/upload/0").exists());
}

#[tokio::test]
async fn oversized_and_truncated_uploads_are_not_published() {
    let directory = TempDir::new().unwrap();
    let state = Arc::new(state(directory.path(), 64, 8).await);
    let address = server(state.clone()).await;
    assert_eq!(
        chunk(address, "upload", "0", b"123456789").await.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let mut body = multipart(&[], b"abcd");
    body.truncate(body.len() - b"\r\n--test-boundary--\r\n".len());
    let response = Client::new()
        .post(format!("http://{address}/api/filebox/upload"))
        .header(
            "Content-Type",
            "multipart/form-data; boundary=test-boundary",
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    idle(&state).await;
    assert_eq!(state.cache_usage.get(), 0);
    assert_eq!(calculate_payload_usage(directory.path()).await.unwrap(), 0);
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM filebox_files")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
}

#[tokio::test]
async fn filebox_key_is_enforced_and_public_settings_explain_requirements() {
    let directory = TempDir::new().unwrap();
    let mut state = state(directory.path(), 64, 8).await;
    state.api_key = Some("test-secret".into());
    let address = server(Arc::new(state)).await;
    assert_eq!(
        chunk(address, "upload", "0", b"test").await.status(),
        StatusCode::UNAUTHORIZED
    );
    let response = Client::new()
        .post(format!("http://{address}/api/filebox/upload-chunk"))
        .header("X-API-Key", "test-secret")
        .header(
            "Content-Type",
            "multipart/form-data; boundary=test-boundary",
        )
        .body(multipart(
            &[("upload_id", "upload"), ("chunk_index", "0")],
            b"test",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = Client::new()
        .get(format!("http://{address}/api/filebox/files"))
        .send()
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(body["auth_required"], true);
    assert_eq!(body["max_file_size"], 8);
}

#[tokio::test]
async fn direct_upload_publishes_complete_files_and_rolls_back_quota_failures() {
    let directory = TempDir::new().unwrap();
    let state = Arc::new(state(directory.path(), 8, 0).await);
    let address = server(state.clone()).await;
    let upload = |bytes: &[u8]| {
        Client::new()
            .post(format!("http://{address}/api/filebox/upload"))
            .header(
                "Content-Type",
                "multipart/form-data; boundary=test-boundary",
            )
            .body(multipart(&[], bytes))
    };
    let response = upload(b"abcdef").send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    let id = body["files"][0]["id"].as_str().unwrap();
    let download = Client::new()
        .get(format!("http://{address}/api/filebox/download/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(download.text().await.unwrap(), "abcdef");
    assert_eq!(
        upload(b"1234").send().await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(state.cache_usage.get(), 6);
    assert_eq!(calculate_payload_usage(directory.path()).await.unwrap(), 6);
}

#[tokio::test]
async fn remote_upload_stream_checks_limits_and_preserves_success_response() {
    let upstream = router_server(
        Router::new()
            .route("/file", get(|| async { "abcdef" }))
            .route(
                "/large",
                get(|| async {
                    Body::from_stream(futures_util::stream::iter(vec![Ok::<_, std::io::Error>(
                        "123456789",
                    )]))
                }),
            ),
    )
    .await;
    let directory = TempDir::new().unwrap();
    let mut state = state(directory.path(), 64, 8).await;
    state.client = safe_client_builder()
        .resolve("upstream.test", upstream)
        .build()
        .unwrap();
    let state = Arc::new(state);
    let address = server(state.clone()).await;
    assert_eq!(
        post(
            address,
            "remote-upload",
            json!({"url":"http://upstream.test/large"})
        )
        .await
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(state.cache_usage.get(), 0);
    let response = post(
        address,
        "remote-upload",
        json!({"url":"http://upstream.test/file"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(body["file"]["file_size"], 6);
    assert_eq!(state.cache_usage.get(), 6);
    assert_eq!(calculate_payload_usage(directory.path()).await.unwrap(), 6);
}

async fn upstream(hits: Arc<AtomicUsize>) -> SocketAddr {
    router_server(Router::new().route(
        "/file",
        get(move || {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                "abcdef"
            }
        }),
    ))
    .await
}

#[tokio::test]
async fn concurrent_downloads_share_one_cache_writer_and_account_bytes() {
    let directory = TempDir::new().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = upstream(hits.clone()).await;
    let mut state = state(directory.path(), 64, 0).await;
    state.client = safe_client_builder()
        .resolve("upstream.test", upstream)
        .build()
        .unwrap();
    let state = Arc::new(state);
    let address = server(state.clone()).await;
    let url = format!("http://{address}/api/proxy?url=http%3A%2F%2Fupstream.test%2Ffile");
    futures_util::future::join_all((0..8).map(|_| {
        let url = url.clone();
        async move {
            assert_eq!(
                Client::new()
                    .get(url)
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "abcdef"
            );
        }
    }))
    .await;
    idle(&state).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(state.cache_usage.get(), 6);
    assert_eq!(calculate_payload_usage(directory.path()).await.unwrap(), 6);
}

#[tokio::test]
async fn insufficient_cache_space_still_allows_download_without_partial_cache() {
    let directory = TempDir::new().unwrap();
    let upstream = upstream(Arc::new(AtomicUsize::new(0))).await;
    let mut state = state(directory.path(), 4, 0).await;
    state.client = safe_client_builder()
        .resolve("upstream.test", upstream)
        .build()
        .unwrap();
    let state = Arc::new(state);
    let address = server(state.clone()).await;
    let response = Client::new()
        .get(format!(
            "http://{address}/api/proxy?url=http%3A%2F%2Fupstream.test%2Ffile"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "abcdef");
    idle(&state).await;
    assert_eq!(state.cache_usage.get(), 0);
    assert_eq!(calculate_payload_usage(directory.path()).await.unwrap(), 0);
}

#[tokio::test]
async fn remote_upload_failures_release_reserved_bytes() {
    let upstream = router_server(Router::new().route(
        "/file",
        get(|| async {
            Body::from_stream(futures_util::stream::iter(vec![
                Ok::<_, std::io::Error>("abcdef"),
                Err(std::io::Error::other("broken upstream")),
            ]))
            .into_response()
        }),
    ))
    .await;
    let directory = TempDir::new().unwrap();
    let mut state = state(directory.path(), 64, 0).await;
    state.client = safe_client_builder()
        .resolve("upstream.test", upstream)
        .build()
        .unwrap();
    let state = Arc::new(state);
    let address = server(state.clone()).await;
    assert_eq!(
        post(
            address,
            "remote-upload",
            json!({"url":"http://upstream.test/file"})
        )
        .await
        .status(),
        StatusCode::BAD_GATEWAY
    );
    idle(&state).await;
    assert_eq!(state.cache_usage.get(), 0);
    assert_eq!(calculate_payload_usage(directory.path()).await.unwrap(), 0);
}

#[tokio::test]
async fn redirect_to_private_network_is_rejected_for_get_head_and_remote_upload() {
    let local_hits = Arc::new(AtomicUsize::new(0));
    let local = upstream(local_hits.clone()).await;
    let redirect = router_server(Router::new().route(
        "/redirect",
        get(move || async move { Redirect::temporary(&format!("http://{local}/file")) }),
    ))
    .await;
    let directory = TempDir::new().unwrap();
    let mut state = state(directory.path(), 64, 0).await;
    state.client = safe_client_builder()
        .resolve("upstream.test", redirect)
        .build()
        .unwrap();
    let address = server(Arc::new(state)).await;
    let url = format!("http://{address}/api/proxy?url=http%3A%2F%2Fupstream.test%2Fredirect");
    assert_eq!(
        Client::new().get(&url).send().await.unwrap().status(),
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(
        Client::new().head(&url).send().await.unwrap().status(),
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(
        post(
            address,
            "remote-upload",
            json!({"url":"http://upstream.test/redirect"})
        )
        .await
        .status(),
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(local_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn spa_pages_work_under_base_path_and_reject_unsafe_forwarded_prefixes() {
    let directory = TempDir::new().unwrap();
    let mut state = state(directory.path(), 64, 0).await;
    state.base_path = "/tools/yundo".into();
    state.frontend_dist = directory.path().join("site");
    fs::create_dir(&state.frontend_dist).await.unwrap();
    fs::write(
        state.frontend_dist.join("index.html"),
        "<html><head></head><body>app</body></html>",
    )
    .await
    .unwrap();
    let address = server(Arc::new(state)).await;
    for page in ["privacy", "terms", "filebox"] {
        let response = Client::new()
            .get(format!("http://{address}/tools/yundo/{page}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .text()
            .await
            .unwrap()
            .contains("window.__YUNDO_BASE_PATH__ = \"/tools/yundo\""));
    }
    let response = Client::new()
        .get(format!("http://{address}/tools/yundo/"))
        .header("X-Forwarded-Prefix", "/\"><script>alert(1)</script>")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = response.text().await.unwrap();
    assert!(!html.contains("alert(1)"));
    assert!(html.contains("window.__YUNDO_BASE_PATH__ = \"/tools/yundo\""));
}
