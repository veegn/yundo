use metrics_exporter_prometheus::PrometheusHandle;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex, Weak,
    },
};
use tokio::fs;
use tokio::sync::{Mutex, RwLock};

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

/// JSON shape returned by `/api/recent`.
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

/// Atomic counter for tracking cache usage.
pub struct CacheUsageTracker {
    /// Current estimated usage in bytes
    current: AtomicU64,
    /// Last calibration timestamp (for periodic recalibration)
    last_calibration: Mutex<std::time::Instant>,
    /// Writers hold a read guard; disk reconciliation only runs while idle.
    pub operations: Arc<RwLock<()>>,
    keys: StdMutex<HashMap<String, Weak<RwLock<()>>>>,
}

impl Default for CacheUsageTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl CacheUsageTracker {
    pub fn new() -> Self {
        Self {
            current: AtomicU64::new(0),
            last_calibration: Mutex::new(std::time::Instant::now()),
            operations: Arc::new(RwLock::new(())),
            keys: StdMutex::new(HashMap::new()),
        }
    }

    pub fn get(&self) -> u64 {
        self.current.load(Ordering::Relaxed)
    }

    pub fn add(&self, bytes: u64) {
        self.current.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn sub(&self, bytes: u64) {
        let _ = self
            .current
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_sub(bytes))
            });
    }

    pub fn try_reserve(&self, bytes: u64, limit: u64) -> Result<(), &'static str> {
        self.current
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(bytes).filter(|next| *next <= limit)
            })
            .map(|_| ())
            .map_err(|_| "Storage space insufficient")
    }

    pub fn key_lock(&self, key: &str) -> Arc<RwLock<()>> {
        let mut keys = self.keys.lock().expect("storage key lock poisoned");
        keys.retain(|_, value| value.strong_count() > 0);
        if let Some(lock) = keys.get(key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(RwLock::new(()));
        keys.insert(key.to_string(), Arc::downgrade(&lock));
        lock
    }

    pub fn set(&self, bytes: u64) {
        self.current.store(bytes, Ordering::Relaxed);
    }

    pub async fn should_recalibrate(&self) -> bool {
        let last = self.last_calibration.lock().await;
        last.elapsed().as_secs() > 300 // Recalibrate every 5 minutes
    }

    pub async fn mark_calibrated(&self) {
        let mut last = self.last_calibration.lock().await;
        *last = std::time::Instant::now();
    }
}

/// Shared application state injected into every Axum handler.
pub struct AppState {
    pub client: Client,
    pub cache_dir: PathBuf,
    pub max_cache_size: u64,
    pub max_file_size: u64,
    pub filebox_size: u64,
    pub db: SqlitePool,
    pub frontend_dist: PathBuf,
    pub base_path: String,
    pub cache_usage: Arc<CacheUsageTracker>,
    pub api_key: Option<String>,
    pub metrics_handle: Option<PrometheusHandle>,
    pub shutdown_token: tokio_util::sync::CancellationToken,
}

pub async fn initialize_cache_dir(cache_dir: &Path) {
    fs::create_dir_all(cache_dir)
        .await
        .expect("failed to create cache directory");
    fs::create_dir_all(cache_dir.join("filebox"))
        .await
        .expect("failed to create filebox directory");
    fs::create_dir_all(cache_dir.join("filebox_tmp"))
        .await
        .expect("failed to create filebox_tmp directory");
}

pub async fn cleanup_temp_files(cache_dir: &Path) {
    tracing::info!("Cleaning up temporary files from previous runs...");

    // Interrupted proxy downloads and staged FileBox uploads are never published.
    let mut count = 0;
    for directory in [
        cache_dir.to_path_buf(),
        cache_dir.join("filebox"),
        cache_dir.join("filebox_tmp"),
    ] {
        if let Ok(mut entries) = fs::read_dir(directory).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext == "tmp" && fs::remove_file(&path).await.is_ok() {
                        count += 1;
                        tracing::debug!("Removed temporary file: {}", path.display());
                    }
                }
            }
        }
    }

    if count > 0 {
        tracing::info!("Cleaned up {} temporary files", count);
    }
}

pub async fn initialize_database(cache_dir: &Path) -> SqlitePool {
    let db_path = cache_dir.join("proxy.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

    let pool = SqlitePoolOptions::new()
        .max_connections(crate::constants::DB_CONNECTION_POOL_SIZE)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_reservations_cannot_exceed_quota_or_wrap() {
        let tracker = Arc::new(CacheUsageTracker::new());
        let barrier = Arc::new(std::sync::Barrier::new(32));
        let threads: Vec<_> = (0..32)
            .map(|_| {
                let tracker = tracker.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    tracker.try_reserve(1, 8).is_ok()
                })
            })
            .collect();
        let accepted = threads
            .into_iter()
            .filter_map(|thread| thread.join().unwrap().then_some(()))
            .count();
        assert_eq!(accepted, 8);
        assert_eq!(tracker.get(), 8);
        tracker.sub(100);
        assert_eq!(tracker.get(), 0);
        tracker.set(u64::MAX);
        assert!(tracker.try_reserve(1, u64::MAX).is_err());
    }
}
