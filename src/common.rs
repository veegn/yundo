use axum::{
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use clap::Parser;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
};
use tokio::fs;
use url::Url;

pub const ALLOWED_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-disposition",
    "accept-ranges",
    "content-range",
    "etag",
    "last-modified",
];

#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Args {
    #[arg(short, long, default_value = "./cache")]
    pub cache_dir: PathBuf,

    #[arg(short = 's', long, value_parser = parse_cache_size)]
    pub cache_size: u64,

    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,

    #[arg(short, long, default_value_t = 8080)]
    pub port: u16,

    #[arg(long, default_value = "./frontend/dist")]
    pub frontend_dist: PathBuf,
}

#[derive(Deserialize)]
pub struct ProxyQuery {
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CacheMeta {
    pub headers: HashMap<String, String>,
    pub status: u16,
}

#[derive(Serialize)]
pub struct HistoryItem {
    pub url: String,
    pub file_name: String,
    pub file_size: i64,
    pub last_download_at: String,
    pub count_7d: i64,
    pub score: f64,
}

pub struct AppState {
    pub client: Client,
    pub cache_dir: PathBuf,
    pub max_cache_size: u64,
    pub db: SqlitePool,
}

pub fn parse_cache_size(input: &str) -> Result<u64, String> {
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

pub fn parse_socket_addr(host: &str, port: u16) -> SocketAddr {
    let ip: IpAddr = host.parse().unwrap_or_else(|_| {
        panic!("invalid host value: {host}");
    });
    SocketAddr::new(ip, port)
}

pub async fn initialize_cache_dir(cache_dir: &Path) {
    fs::create_dir_all(cache_dir)
        .await
        .expect("failed to create cache directory");
}

pub async fn initialize_database(cache_dir: &Path) -> SqlitePool {
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

pub fn ensure_download_filename(headers: &mut HeaderMap, file_name: &str) {
    if headers.contains_key("content-disposition") {
        return;
    }

    if let Ok(value) = HeaderValue::from_str(&build_content_disposition(file_name)) {
        headers.insert("content-disposition", value);
    }
}

pub fn resolve_file_name(
    original_url: &Url,
    final_url: Option<&Url>,
    headers: &HeaderMap,
) -> String {
    extract_filename_from_headers(headers)
        .or_else(|| final_url.and_then(extract_filename_from_url))
        .or_else(|| extract_filename_from_url(original_url))
        .unwrap_or_else(|| "download.bin".to_string())
}

pub fn extract_filename_from_url(url: &Url) -> Option<String> {
    for (key, value) in url.query_pairs() {
        let key = key.to_ascii_lowercase();
        if (key == "response-content-disposition" || key == "rscd")
            && parse_content_disposition_filename(&value).is_some()
        {
            return parse_content_disposition_filename(&value);
        }
    }

    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
}

fn extract_filename_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("content-disposition")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_disposition_filename)
}

fn parse_content_disposition_filename(value: &str) -> Option<String> {
    for part in value.split(';').map(str::trim) {
        if let Some(encoded) = part.strip_prefix("filename*=") {
            let encoded = encoded.strip_prefix("UTF-8''").unwrap_or(encoded);
            if let Ok(decoded) = percent_decode(encoded) {
                let sanitized = sanitize_ascii_filename(&decoded);
                if !sanitized.is_empty() {
                    return Some(decoded);
                }
            }
        }

        if let Some(name) = part.strip_prefix("filename=") {
            let trimmed = name.trim_matches('"').trim_matches('\'').trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

fn percent_decode(value: &str) -> Result<String, ()> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).map_err(|_| ())?;
                let byte = u8::from_str_radix(hex, 16).map_err(|_| ())?;
                decoded.push(byte);
                index += 3;
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8(decoded).map_err(|_| ())
}

pub fn build_content_disposition(file_name: &str) -> String {
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

pub async fn health_handler() -> &'static str {
    "ok"
}

pub async fn root_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        "Frontend assets are not built yet. Run `npm install` then `npm run build --workspace=frontend`, or call the API routes directly.",
    )
}

pub fn is_forbidden_host(host: &str) -> bool {
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
