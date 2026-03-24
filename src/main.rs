use axum::{
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use bytes::Bytes;
use clap::Parser;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};
use tokio::{
    fs::{self, File},
    io::AsyncWriteExt,
    sync::mpsc,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use url::Url;

const ALLOWED_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-disposition",
    "accept-ranges",
    "content-range",
];

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Cache directory path
    #[arg(short, long, default_value = "./cache")]
    cache_dir: PathBuf,

    /// Maximum cache size. Supports plain bytes or units like 512MB, 2GB, 1GiB.
    #[arg(short = 's', long, value_parser = parse_cache_size)]
    cache_size: u64,

    /// HTTP bind host
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// HTTP bind port
    #[arg(short, long, default_value_t = 8080)]
    port: u16,

    /// Built frontend assets directory
    #[arg(long, default_value = "./frontend/dist")]
    frontend_dist: PathBuf,
}

#[derive(Deserialize)]
struct ProxyQuery {
    url: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct CacheMeta {
    headers: HashMap<String, String>,
    status: u16,
}

#[derive(Serialize)]
struct HistoryItem {
    url: String,
    file_name: String,
    file_size: i64,
    last_download_at: String,
    count_7d: i64,
    score: f64,
}

struct AppState {
    client: Client,
    cache_dir: PathBuf,
    max_cache_size: u64,
    db: SqlitePool,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    initialize_cache_dir(&args.cache_dir).await;
    let db = initialize_database(&args.cache_dir).await;
    let max_cache_size = args.cache_size;

    tracing::info!("cache_dir = {}", args.cache_dir.display());
    tracing::info!("frontend_dist = {}", args.frontend_dist.display());
    tracing::info!("max_cache_size = {}", max_cache_size);

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .expect("failed to build HTTP client");

    let state = Arc::new(AppState {
        client,
        cache_dir: args.cache_dir.clone(),
        max_cache_size,
        db: db.clone(),
    });

    spawn_cache_eviction_task(state.clone());
    spawn_history_cleanup_task(db.clone());

    let api_router = Router::new()
        .route("/api/proxy", get(proxy_handler))
        .route("/api/recent", get(history_handler))
        .route("/api/history", get(history_handler))
        .route("/healthz", get(health_handler))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let app = if args.frontend_dist.join("index.html").exists() {
        tracing::info!("serving frontend assets from {}", args.frontend_dist.display());
        api_router.fallback_service(
            ServeDir::new(&args.frontend_dist)
                .not_found_service(ServeFile::new(args.frontend_dist.join("index.html"))),
        )
    } else {
        tracing::warn!(
            "frontend dist missing at {}, only API routes will be available",
            args.frontend_dist.display()
        );
        api_router.route("/", get(root_handler))
    };

    let addr = parse_socket_addr(&args.host, args.port);
    tracing::info!("listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");
    axum::serve(listener, app)
        .await
        .expect("failed to serve application");
}

fn parse_cache_size(input: &str) -> Result<u64, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("cache size cannot be empty".to_string());
    }

    let split_at = trimmed
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (number_part, unit_part) = trimmed.split_at(split_at);

    if number_part.is_empty() {
        return Err(format!(
            "invalid cache size `{input}`; expected formats like 1073741824, 512MB, 2GB, 1GiB"
        ));
    }

    let value = number_part
        .parse::<u64>()
        .map_err(|_| format!("invalid numeric cache size `{input}`"))?;

    let multiplier = match unit_part.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1_u64,
        "k" | "kb" => 1_000_u64,
        "m" | "mb" => 1_000_000_u64,
        "g" | "gb" => 1_000_000_000_u64,
        "t" | "tb" => 1_000_000_000_000_u64,
        "kib" => 1_024_u64,
        "mib" => 1_024_u64.pow(2),
        "gib" => 1_024_u64.pow(3),
        "tib" => 1_024_u64.pow(4),
        _ => {
            return Err(format!(
                "unsupported cache size unit in `{input}`; supported units: B, KB, MB, GB, TB, KiB, MiB, GiB, TiB"
            ))
        }
    };

    value
        .checked_mul(multiplier)
        .ok_or_else(|| format!("cache size `{input}` is too large"))
}

fn parse_socket_addr(host: &str, port: u16) -> SocketAddr {
    let ip: IpAddr = host.parse().unwrap_or_else(|_| {
        panic!("invalid host value: {host}");
    });
    SocketAddr::new(ip, port)
}

async fn initialize_cache_dir(cache_dir: &Path) {
    fs::create_dir_all(cache_dir)
        .await
        .expect("failed to create cache directory");
}

async fn initialize_database(cache_dir: &Path) -> SqlitePool {
    let db_path = cache_dir.join("proxy.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("failed to connect to SQLite");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS download_history (
            url TEXT PRIMARY KEY,
            file_name TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            last_download_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize download_history");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS download_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT NOT NULL,
            downloaded_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize download_events");

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_url ON download_events(url)")
        .execute(&pool)
        .await
        .expect("failed to initialize idx_events_url");

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_time ON download_events(downloaded_at)")
        .execute(&pool)
        .await
        .expect("failed to initialize idx_events_time");

    pool
}

fn spawn_cache_eviction_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            if let Err(err) = enforce_cache_size(&state.cache_dir, state.max_cache_size).await {
                tracing::error!("cache eviction failed: {err}");
            }
        }
    });
}

fn spawn_history_cleanup_task(db: SqlitePool) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            if let Err(err) = sqlx::query(
                "DELETE FROM download_events
                 WHERE downloaded_at < datetime('now', '-7 days')",
            )
            .execute(&db)
            .await
            {
                tracing::warn!("cleanup old download events failed: {err}");
            }
        }
    });
}

fn ensure_download_filename(headers: &mut HeaderMap, file_name: &str) {
    if headers.contains_key("content-disposition") {
        return;
    }

    if let Ok(value) = HeaderValue::from_str(&build_content_disposition(file_name)) {
        headers.insert("content-disposition", value);
    }
}

fn build_content_disposition(file_name: &str) -> String {
    let ascii_name = sanitize_ascii_filename(file_name);
    let encoded_name = percent_encode_utf8(file_name);
    format!("attachment; filename=\"{ascii_name}\"; filename*=UTF-8''{encoded_name}")
}

fn sanitize_ascii_filename(file_name: &str) -> String {
    let sanitized = file_name
        .chars()
        .map(|ch| match ch {
            '"' | '\\' | '/' | ':' | '*' | '?' | '<' | '>' | '|' => '_',
            c if c.is_ascii_graphic() || c == ' ' => c,
            _ => '_',
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string();

    if sanitized.is_empty() {
        "download.bin".to_string()
    } else {
        sanitized
    }
}

fn percent_encode_utf8(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());

    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~' => encoded.push(*byte as char),
            _ => encoded.push_str(&format!("%{:02X}", byte)),
        }
    }

    encoded
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn root_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        "Frontend assets are not built yet. Run `npm install` then `npm run build --workspace=frontend`, or call the API routes directly.",
    )
}

async fn enforce_cache_size(cache_dir: &Path, max_size: u64) -> std::io::Result<()> {
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

    files.sort_by_key(|(_, _, modified)| *modified);

    for (path, size, _) in files {
        if total_size <= max_size {
            break;
        }

        if fs::remove_file(&path).await.is_ok() {
            total_size -= size;
        }
    }

    Ok(())
}

async fn history_handler(State(state): State<Arc<AppState>>) -> Json<Vec<HistoryItem>> {
    let rows = sqlx::query(
        "SELECT
            h.url,
            h.file_name,
            h.file_size,
            h.last_download_at,
            (
                SELECT COUNT(*)
                FROM download_events e
                WHERE e.url = h.url
                  AND e.downloaded_at >= datetime('now', '-7 days')
            ) AS count_7d,
            (julianday('now') - julianday(h.last_download_at)) * 24 AS hours_since_last
        FROM download_history h",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut items = rows
        .into_iter()
        .map(|row| {
            let count_7d: i64 = row.get("count_7d");
            let hours_since_last: f64 = row.get("hours_since_last");
            let score = ((count_7d as f64 + 1.0).powf(0.8)) / ((hours_since_last + 2.0).powf(1.5));

            HistoryItem {
                url: row.get("url"),
                file_name: row.get("file_name"),
                file_size: row.get("file_size"),
                last_download_at: row.get("last_download_at"),
                count_7d,
                score,
            }
        })
        .collect::<Vec<_>>();

    items.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items.truncate(50);

    Json(items)
}

async fn record_download(db: SqlitePool, url: String, file_name: String, file_size: i64) {
    tokio::spawn(async move {
        if let Err(err) = sqlx::query(
            "INSERT INTO download_history (url, file_name, file_size, last_download_at)
             VALUES (?, ?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(url) DO UPDATE SET
               file_name = excluded.file_name,
               file_size = excluded.file_size,
               last_download_at = CURRENT_TIMESTAMP",
        )
        .bind(&url)
        .bind(&file_name)
        .bind(file_size)
        .execute(&db)
        .await
        {
            tracing::warn!("failed to update download history: {err}");
            return;
        }

        if let Err(err) = sqlx::query("INSERT INTO download_events (url) VALUES (?)")
            .bind(&url)
            .execute(&db)
            .await
        {
            tracing::warn!("failed to insert download event: {err}");
        }
    });
}

async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProxyQuery>,
    req_headers: HeaderMap,
) -> impl IntoResponse {
    let target_url = query.url;
    let parsed_url = match Url::parse(&target_url) {
        Ok(url) => url,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid URL format").into_response(),
    };

    if !matches!(parsed_url.scheme(), "http" | "https") {
        return (StatusCode::BAD_REQUEST, "only HTTP and HTTPS URLs are supported").into_response();
    }

    let host = parsed_url.host_str().unwrap_or_default().to_ascii_lowercase();
    if is_forbidden_host(&host) {
        return (
            StatusCode::FORBIDDEN,
            "access to local or private networks is forbidden",
        )
            .into_response();
    }

    let file_name = parsed_url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|segment| !segment.is_empty())
        .unwrap_or("download.bin")
        .to_string();

    let range_value = req_headers
        .get("range")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");

    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(target_url.as_bytes());
        hasher.update(b"|");
        hasher.update(range_value.as_bytes());
        hex::encode(hasher.finalize())
    };

    let data_path = state.cache_dir.join(format!("{hash}.data"));
    let meta_path = state.cache_dir.join(format!("{hash}.meta"));

    if let Some(response) = try_serve_from_cache(
        &data_path,
        &meta_path,
        state.db.clone(),
        target_url.clone(),
        file_name.clone(),
    )
    .await
    {
        return response;
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
            return (StatusCode::BAD_GATEWAY, "failed to reach target server").into_response();
        }
    };

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

        if ALLOWED_HEADERS.contains(&header_name.as_str()) {
            response_headers.insert(name.clone(), value.clone());
            if let Ok(value_text) = value.to_str() {
                meta_headers.insert(header_name, value_text.to_string());
            }
        }
    }

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

    let cache_meta = CacheMeta {
        headers: meta_headers,
        status: status.as_u16(),
    };
    let should_cache = status.is_success();
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

async fn try_serve_from_cache(
    data_path: &Path,
    meta_path: &Path,
    db: SqlitePool,
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

fn is_forbidden_host(host: &str) -> bool {
    host == "localhost"
        || host == "::1"
        || host == "0.0.0.0"
        || host == "127.0.0.1"
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || is_172_private_range(host)
}

fn is_172_private_range(host: &str) -> bool {
    let Some(rest) = host.strip_prefix("172.") else {
        return false;
    };

    let Some(second_octet) = rest.split('.').next() else {
        return false;
    };

    second_octet
        .parse::<u8>()
        .map(|octet| (16..=31).contains(&octet))
        .unwrap_or(false)
}
