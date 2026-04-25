use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tokio::fs;

/// Query parameter for the proxy endpoint.
#[derive(Deserialize)]
pub struct ProxyQuery {
    pub url: String,
}

/// Serialised cache metadata stored alongside each cached response body.
#[derive(Serialize, Deserialize, Clone)]
pub struct CacheMeta {
    pub headers: HashMap<String, String>,
    pub status: u16,
}

/// JSON shape returned by `/api/recent` and `/api/history`.
#[derive(Serialize)]
pub struct HistoryItem {
    pub slug: String,
    pub url: String,
    pub file_name: String,
    pub file_size: i64,
    pub last_download_at: String,
    pub count_7d: i64,
    pub score: f64,
}

/// Shared application state injected into every Axum handler.
pub struct AppState {
    pub client: Client,
    pub cache_dir: PathBuf,
    pub max_cache_size: u64,
    pub db: SqlitePool,
    pub frontend_dist: PathBuf,
    pub base_path: String,
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
