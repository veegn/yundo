use sqlx::{Row, SqlitePool};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// A cached, read-only view of a storage node for scheduling decisions.
#[derive(Debug, Clone)]
pub struct NodeView {
    pub id: String,
    pub endpoint: String,
    pub zone: Option<String>,
    pub status: String,
    pub capacity_bytes: i64,
    pub used_bytes: i64,
    pub active_uploads: i64,
    pub active_downloads: i64,
    pub active_replications: i64,
    pub avg_rtt_ms: Option<i64>,
    pub p95_rtt_ms: Option<i64>,
    pub packet_loss: Option<f64>,
    pub timeout_rate: Option<f64>,
    pub heartbeat_success_rate: Option<f64>,
    pub features: Option<String>,
    pub last_heartbeat_at: Option<String>,
}

impl NodeView {
    /// Whether the node is considered live (heartbeat not expired).
    pub fn is_live(&self, heartbeat_ttl_secs: u64) -> bool {
        let Some(ref hb) = self.last_heartbeat_at else {
            return false;
        };
        // Parse SQLite datetime format
        let Ok(dt) = chrono::NaiveDateTime::parse_from_str(hb, "%Y-%m-%d %H:%M:%S") else {
            return false;
        };
        let now = chrono::Utc::now().naive_utc();
        let elapsed = now.signed_duration_since(dt);
        elapsed.num_seconds() <= heartbeat_ttl_secs as i64
    }

    /// Free space ratio (0.0 to 1.0).
    pub fn free_ratio(&self) -> f64 {
        if self.capacity_bytes <= 0 {
            return 0.0;
        }
        ((self.capacity_bytes - self.used_bytes) as f64) / (self.capacity_bytes as f64)
    }

    /// Check if the node has a specific feature.
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features
            .as_deref()
            .map(|f| f.contains(feature))
            .unwrap_or(false)
    }
}

/// In-memory cache of the storage node discovery view.
/// Refreshes from DB when TTL expires. Supports manual invalidation.
#[derive(Clone)]
pub struct ServiceDiscovery {
    db: SqlitePool,
    cache: Arc<RwLock<CachedNodes>>,
    ttl: Duration,
}

struct CachedNodes {
    nodes: Vec<NodeView>,
    updated_at: Instant,
}

impl ServiceDiscovery {
    pub fn new(db: SqlitePool, ttl_secs: u64) -> Self {
        Self {
            db,
            cache: Arc::new(RwLock::new(CachedNodes {
                nodes: Vec::new(),
                updated_at: Instant::now() - Duration::from_secs(ttl_secs + 1),
            })),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Get all nodes, refreshing from DB if cache is stale.
    pub async fn get_all_nodes(&self) -> Vec<NodeView> {
        {
            let cache = self.cache.read().await;
            if cache.updated_at.elapsed() < self.ttl {
                return cache.nodes.clone();
            }
        }
        self.refresh().await
    }

    /// Get nodes suitable for writing (active, live, writable).
    pub async fn get_writable_nodes(&self, heartbeat_ttl_secs: u64) -> Vec<NodeView> {
        self.get_all_nodes()
            .await
            .into_iter()
            .filter(|n| {
                n.status == "active"
                    && n.is_live(heartbeat_ttl_secs)
                    && n.has_feature("chunk-write")
            })
            .collect()
    }

    /// Get nodes suitable for reading (active/degraded, live).
    pub async fn get_readable_nodes(&self, heartbeat_ttl_secs: u64) -> Vec<NodeView> {
        self.get_all_nodes()
            .await
            .into_iter()
            .filter(|n| {
                matches!(n.status.as_str(), "active" | "degraded" | "draining")
                    && n.is_live(heartbeat_ttl_secs)
                    && n.has_feature("chunk-read")
            })
            .collect()
    }

    /// Force cache invalidation (called after node status changes).
    pub async fn invalidate(&self) {
        let mut cache = self.cache.write().await;
        cache.updated_at = Instant::now() - self.ttl - Duration::from_secs(1);
    }

    /// Refresh cache from database.
    async fn refresh(&self) -> Vec<NodeView> {
        let rows = sqlx::query(
            "SELECT id, endpoint, zone, status, capacity_bytes, used_bytes,
                    active_uploads, active_downloads, active_replications,
                    avg_rtt_ms, p95_rtt_ms, packet_loss, timeout_rate,
                    heartbeat_success_rate, features, last_heartbeat_at
             FROM storage_nodes
             WHERE status != 'offline'
             ORDER BY id",
        )
        .fetch_all(&self.db)
        .await;

        let nodes: Vec<NodeView> = match rows {
            Ok(rows) => rows
                .iter()
                .map(|row| NodeView {
                    id: row.get("id"),
                    endpoint: row.get("endpoint"),
                    zone: row.get("zone"),
                    status: row.get("status"),
                    capacity_bytes: row.get("capacity_bytes"),
                    used_bytes: row.get("used_bytes"),
                    active_uploads: row.get("active_uploads"),
                    active_downloads: row.get("active_downloads"),
                    active_replications: row.get("active_replications"),
                    avg_rtt_ms: row.get("avg_rtt_ms"),
                    p95_rtt_ms: row.get("p95_rtt_ms"),
                    packet_loss: row.get("packet_loss"),
                    timeout_rate: row.get("timeout_rate"),
                    heartbeat_success_rate: row.get("heartbeat_success_rate"),
                    features: row.get("features"),
                    last_heartbeat_at: row.get("last_heartbeat_at"),
                })
                .collect(),
            Err(err) => {
                tracing::error!("failed to refresh service discovery: {err}");
                Vec::new()
            }
        };

        let mut cache = self.cache.write().await;
        cache.nodes = nodes.clone();
        cache.updated_at = Instant::now();
        nodes
    }
}
