use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use precision_proxy::{
    app::build_router,
    common::{initialize_cache_dir, initialize_database, parse_cache_size, AppState},
};
use reqwest::Client;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::{fs, net::TcpListener, time::sleep};
use url::Url;

#[tokio::test]
async fn parse_cache_size_supports_human_readable_units() {
    assert_eq!(parse_cache_size("512MB").unwrap(), 512_000_000);
    assert_eq!(parse_cache_size("2GB").unwrap(), 2_000_000_000);
    assert_eq!(parse_cache_size("1GiB").unwrap(), 1_073_741_824);
    assert!(parse_cache_size("12XB").is_err());
}

#[tokio::test]
async fn proxy_head_returns_filename_from_signed_url_query() {
    let cache_dir = TempDir::new().unwrap();
    let upstream = spawn_upstream_server(Arc::new(AtomicUsize::new(0))).await;
    let app = spawn_proxy_server(cache_dir.path().to_path_buf(), upstream).await;

    let mut signed_url = Url::parse("http://upstream.test/file").unwrap();
    signed_url
        .query_pairs_mut()
        .append_pair(
            "response-content-disposition",
            "attachment; filename=Clash.Verge_2.4.7_x64-setup.exe",
        );

    let response = Client::new()
        .head(format!(
            "http://{}/api/proxy?url={}",
            app,
            urlencoding::encode(signed_url.as_str())
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap(),
        "attachment; filename=\"Clash.Verge_2.4.7_x64-setup.exe\"; filename*=UTF-8''Clash.Verge_2.4.7_x64-setup.exe"
    );
}

#[tokio::test]
async fn full_download_requests_are_served_from_cache() {
    let cache_dir = TempDir::new().unwrap();
    let upstream_hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_upstream_server(upstream_hits.clone()).await;
    let app = spawn_proxy_server(cache_dir.path().to_path_buf(), upstream).await;

    let proxy_url = format!(
        "http://{}/api/proxy?url={}",
        app,
        urlencoding::encode("http://upstream.test/file")
    );

    let client = Client::new();
    let first = client.get(&proxy_url).send().await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.text().await.unwrap(), "abcdef");

    sleep(Duration::from_millis(200)).await;

    let second = client.get(&proxy_url).send().await.unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(second.text().await.unwrap(), "abcdef");

    assert_eq!(upstream_hits.load(Ordering::SeqCst), 1);
    assert_eq!(count_cache_data_files(cache_dir.path()).await, 1);
}

#[tokio::test]
async fn range_requests_bypass_cache() {
    let cache_dir = TempDir::new().unwrap();
    let upstream_hits = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_upstream_server(upstream_hits.clone()).await;
    let app = spawn_proxy_server(cache_dir.path().to_path_buf(), upstream).await;

    let proxy_url = format!(
        "http://{}/api/proxy?url={}",
        app,
        urlencoding::encode("http://upstream.test/file")
    );

    let client = Client::new();
    let first = client
        .get(&proxy_url)
        .header("Range", "bytes=1-3")
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(first.text().await.unwrap(), "bcd");

    sleep(Duration::from_millis(100)).await;

    let second = client
        .get(&proxy_url)
        .header("Range", "bytes=1-3")
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(second.text().await.unwrap(), "bcd");

    assert_eq!(upstream_hits.load(Ordering::SeqCst), 2);
    assert_eq!(count_cache_data_files(cache_dir.path()).await, 0);
}

async fn spawn_proxy_server(cache_dir: PathBuf, upstream_addr: SocketAddr) -> SocketAddr {
    initialize_cache_dir(&cache_dir).await;
    let db = initialize_database(&cache_dir).await;
    let state = Arc::new(AppState {
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .resolve("upstream.test", upstream_addr)
            .build()
            .unwrap(),
        cache_dir,
        max_cache_size: 64 * 1024 * 1024,
        db,
    });

    let router = build_router(state, PathBuf::from("./frontend/missing-dist"));
    spawn_router(router).await
}

async fn spawn_upstream_server(hit_count: Arc<AtomicUsize>) -> SocketAddr {
    async fn get_file(headers: HeaderMap, hit_count: Arc<AtomicUsize>) -> impl IntoResponse {
        hit_count.fetch_add(1, Ordering::SeqCst);

        let body = b"abcdef";
        let mut response_headers = HeaderMap::new();
        response_headers.insert("accept-ranges", HeaderValue::from_static("bytes"));
        response_headers.insert("etag", HeaderValue::from_static("\"test-etag\""));
        response_headers.insert("last-modified", HeaderValue::from_static("Wed, 26 Mar 2026 10:00:00 GMT"));

        if let Some(range) = headers.get("range").and_then(|value| value.to_str().ok()) {
            if range == "bytes=1-3" {
                response_headers.insert("content-range", HeaderValue::from_static("bytes 1-3/6"));
                response_headers.insert("content-length", HeaderValue::from_static("3"));
                return (StatusCode::PARTIAL_CONTENT, response_headers, Body::from("bcd"));
            }
        }

        response_headers.insert("content-length", HeaderValue::from_static("6"));
        (StatusCode::OK, response_headers, Body::from(body.as_slice()))
    }

    async fn head_file() -> impl IntoResponse {
        let mut response_headers = HeaderMap::new();
        response_headers.insert("accept-ranges", HeaderValue::from_static("bytes"));
        response_headers.insert("etag", HeaderValue::from_static("\"test-etag\""));
        response_headers.insert("last-modified", HeaderValue::from_static("Wed, 26 Mar 2026 10:00:00 GMT"));
        response_headers.insert("content-length", HeaderValue::from_static("6"));
        (StatusCode::OK, response_headers)
    }

    let app = Router::new().route(
        "/file",
        get({
            let hit_count = hit_count.clone();
            move |headers| get_file(headers, hit_count.clone())
        })
        .head(head_file),
    );

    spawn_router(app).await
}

async fn spawn_router(app: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

async fn count_cache_data_files(cache_dir: &Path) -> usize {
    let mut count = 0;
    let mut entries = fs::read_dir(cache_dir).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        if entry
            .path()
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "data")
        {
            count += 1;
        }
    }
    count
}
