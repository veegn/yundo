use crate::config::NodeMode;
use crate::storage::StorageBackend;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
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

/// Node-specific configuration for storage/all modes.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub node_id: String,
    pub endpoint: Option<String>,
    pub zone: Option<String>,
    pub api_endpoint: Option<String>,
    pub heartbeat_interval_secs: u64,
    pub heartbeat_ttl_secs: u64,
    pub default_chunk_size: i64,
    pub default_replication_factor: i64,
}

/// Shared application state injected into every Axum handler.
pub struct AppState {
    pub client: Client,
    pub cache_dir: PathBuf,
    pub max_cache_size: u64,
    pub filebox_size: u64,
    pub db: SqlitePool,
    pub frontend_dist: PathBuf,
    pub base_path: String,
    pub storage_backend: Arc<dyn StorageBackend>,
    pub node_mode: NodeMode,
    pub node_config: NodeConfig,
    pub internal_token: Option<String>,
}

pub async fn initialize_cache_dir(cache_dir: &Path) {
    fs::create_dir_all(cache_dir)
        .await
        .expect("failed to create cache directory");
    fs::create_dir_all(cache_dir.join("filebox"))
        .await
        .expect("failed to create filebox directory");
    fs::create_dir_all(cache_dir.join("storage"))
        .await
        .expect("failed to create storage directory");
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
        "CREATE TABLE IF NOT EXISTS filebox_files (
            id TEXT PRIMARY KEY,
            file_name TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            uploaded_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            expires_at DATETIME NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize filebox_files");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS files (
            id TEXT PRIMARY KEY,
            file_name TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            content_type TEXT,
            chunk_size INTEGER NOT NULL,
            total_chunks INTEGER NOT NULL,
            sha256 TEXT,
            status TEXT NOT NULL DEFAULT 'uploading',
            replication_factor INTEGER NOT NULL DEFAULT 1,
            uploaded_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            expires_at DATETIME,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize files");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS file_chunks (
            id TEXT PRIMARY KEY,
            file_id TEXT NOT NULL,
            chunk_index INTEGER NOT NULL,
            size_bytes INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(file_id, chunk_index)
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize file_chunks");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS chunk_replicas (
            chunk_id TEXT NOT NULL,
            node_id TEXT NOT NULL,
            object_key TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'ready',
            verified_at DATETIME,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (chunk_id, node_id)
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize chunk_replicas");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS upload_sessions (
            id TEXT PRIMARY KEY,
            file_id TEXT NOT NULL,
            file_name TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            content_type TEXT,
            chunk_size INTEGER NOT NULL,
            total_chunks INTEGER NOT NULL,
            replication_factor INTEGER NOT NULL DEFAULT 1,
            status TEXT NOT NULL DEFAULT 'active',
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            expires_at DATETIME NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize upload_sessions");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS upload_session_chunks (
            upload_id TEXT NOT NULL,
            chunk_index INTEGER NOT NULL,
            size_bytes INTEGER,
            sha256 TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            node_id TEXT,
            object_key TEXT,
            error TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (upload_id, chunk_index)
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize upload_session_chunks");

    // --- Phase 3: Storage Nodes ---
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS storage_nodes (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            zone TEXT,
            status TEXT NOT NULL DEFAULT 'registered',
            capacity_bytes INTEGER NOT NULL,
            used_bytes INTEGER NOT NULL DEFAULT 0,
            active_uploads INTEGER NOT NULL DEFAULT 0,
            active_downloads INTEGER NOT NULL DEFAULT 0,
            active_replications INTEGER NOT NULL DEFAULT 0,
            avg_rtt_ms INTEGER,
            p95_rtt_ms INTEGER,
            packet_loss REAL,
            timeout_rate REAL,
            heartbeat_success_rate REAL,
            features TEXT,
            storage_version TEXT,
            public_download INTEGER NOT NULL DEFAULT 0,
            last_heartbeat_at DATETIME,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize storage_nodes");

    // --- Phase 4: GC Tasks ---
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS storage_gc_tasks (
            id TEXT PRIMARY KEY,
            file_id TEXT,
            chunk_id TEXT,
            node_id TEXT NOT NULL,
            object_key TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            retry_count INTEGER NOT NULL DEFAULT 0,
            max_retry INTEGER NOT NULL DEFAULT 10,
            next_retry_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            locked_by TEXT,
            locked_until DATETIME,
            last_error TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize storage_gc_tasks");

    // --- Phase 4: Repair Tasks ---
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS replica_repair_tasks (
            id TEXT PRIMARY KEY,
            file_id TEXT,
            chunk_id TEXT NOT NULL,
            source_node_id TEXT,
            target_node_id TEXT,
            reason TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            priority INTEGER NOT NULL DEFAULT 100,
            retry_count INTEGER NOT NULL DEFAULT 0,
            next_retry_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            locked_by TEXT,
            locked_until DATETIME,
            last_error TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize replica_repair_tasks");

    // --- Indexes ---
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_files_status_expires ON files(status, expires_at)")
        .execute(&pool)
        .await
        .expect("failed to initialize idx_files_status_expires");

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_file_chunks_file_id ON file_chunks(file_id, chunk_index)",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize idx_file_chunks_file_id");

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_chunk_replicas_node ON chunk_replicas(node_id, status)",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize idx_chunk_replicas_node");

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_upload_chunks_status ON upload_session_chunks(upload_id, status)")
        .execute(&pool)
        .await
        .expect("failed to initialize idx_upload_chunks_status");

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_gc_tasks_claim ON storage_gc_tasks(status, next_retry_at, locked_until)",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize idx_gc_tasks_claim");

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_repair_tasks_claim ON replica_repair_tasks(status, priority, next_retry_at, locked_until)",
    )
    .execute(&pool)
    .await
    .expect("failed to initialize idx_repair_tasks_claim");

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
