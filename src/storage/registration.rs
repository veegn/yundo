use crate::common::AppState;
use crate::storage::audit;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use super::auth::extract_bearer_token;

// ---------------------------------------------------------------------------
// Control Plane endpoints (api | all modes)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RegisterNodePayload {
    pub node_id: String,
    pub name: String,
    pub endpoint: String,
    pub zone: Option<String>,
    pub capacity_bytes: i64,
    pub storage_version: Option<String>,
    pub features: Option<Vec<String>>,
    pub public_download: Option<bool>,
}

/// POST /api/storage/nodes/register
pub async fn register_node_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<RegisterNodePayload>,
) -> impl IntoResponse {
    // Validate internal token
    let token = extract_bearer_token(&headers);
    if let Some(ref expected) = state.internal_token {
        match token {
            Some(t) if t == expected.as_str() => {}
            _ => {
                audit::audit(
                    "node_register",
                    &payload.node_id,
                    Some(&payload.node_id),
                    "rejected",
                    Some("invalid token"),
                );
                return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
            }
        }
    }

    // SSRF prevention: validate endpoint
    if let Err(reason) = validate_endpoint(&payload.endpoint) {
        audit::audit(
            "node_register",
            &payload.node_id,
            Some(&payload.node_id),
            "rejected",
            Some(&reason),
        );
        return (StatusCode::BAD_REQUEST, reason).into_response();
    }

    let features_str = payload.features.as_ref().map(|f| f.join(","));
    let public_download = payload.public_download.unwrap_or(false) as i32;

    let result = sqlx::query(
        "INSERT INTO storage_nodes (
            id, name, endpoint, zone, status, capacity_bytes, features,
            storage_version, public_download, updated_at
        ) VALUES (?, ?, ?, ?, 'registered', ?, ?, ?, ?, CURRENT_TIMESTAMP)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            endpoint = excluded.endpoint,
            zone = excluded.zone,
            capacity_bytes = excluded.capacity_bytes,
            features = excluded.features,
            storage_version = excluded.storage_version,
            public_download = excluded.public_download,
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(&payload.node_id)
    .bind(&payload.name)
    .bind(&payload.endpoint)
    .bind(payload.zone.as_deref())
    .bind(payload.capacity_bytes)
    .bind(features_str.as_deref())
    .bind(payload.storage_version.as_deref())
    .bind(public_download)
    .execute(&state.db)
    .await;

    if let Err(err) = result {
        tracing::error!("failed to register node: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    audit::audit(
        "node_register",
        &payload.node_id,
        Some(&payload.node_id),
        "success",
        None,
    );

    let heartbeat_interval = state.node_config.heartbeat_interval_secs;
    let heartbeat_ttl = state.node_config.heartbeat_ttl_secs;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "registered": true,
            "node_id": payload.node_id,
            "heartbeat_interval_secs": heartbeat_interval,
            "heartbeat_ttl_secs": heartbeat_ttl,
            "assigned_status": "registered",
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct HeartbeatPayload {
    pub capacity_bytes: Option<i64>,
    pub used_bytes: Option<i64>,
    pub active_uploads: Option<i64>,
    pub active_downloads: Option<i64>,
    pub active_replications: Option<i64>,
    pub disk_ok: Option<bool>,
    pub avg_rtt_ms: Option<i64>,
    pub p95_rtt_ms: Option<i64>,
    pub packet_loss: Option<f64>,
    pub timeout_rate: Option<f64>,
}

/// POST /api/storage/nodes/:id/heartbeat
pub async fn heartbeat_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(node_id): Path<String>,
    Json(payload): Json<HeartbeatPayload>,
) -> impl IntoResponse {
    // Validate internal token
    let token = extract_bearer_token(&headers);
    if let Some(ref expected) = state.internal_token {
        match token {
            Some(t) if t == expected.as_str() => {}
            _ => return (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
        }
    }

    // Check node exists
    let node_row = sqlx::query("SELECT status FROM storage_nodes WHERE id = ?")
        .bind(&node_id)
        .fetch_optional(&state.db)
        .await;

    let current_status: String = match node_row {
        Ok(Some(row)) => row.get("status"),
        Ok(None) => return (StatusCode::NOT_FOUND, "node not registered").into_response(),
        Err(err) => {
            tracing::error!("heartbeat db error: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    // Status convergence: first heartbeat after registration → active
    let new_status = match current_status.as_str() {
        "registered" => "active",
        "degraded" => "active", // recovered after observation window (simplified)
        other => other,
    };

    let result = sqlx::query(
        "UPDATE storage_nodes SET
            status = ?,
            capacity_bytes = COALESCE(?, capacity_bytes),
            used_bytes = COALESCE(?, used_bytes),
            active_uploads = COALESCE(?, active_uploads),
            active_downloads = COALESCE(?, active_downloads),
            active_replications = COALESCE(?, active_replications),
            avg_rtt_ms = ?,
            p95_rtt_ms = ?,
            packet_loss = ?,
            timeout_rate = ?,
            last_heartbeat_at = CURRENT_TIMESTAMP,
            updated_at = CURRENT_TIMESTAMP
         WHERE id = ?",
    )
    .bind(new_status)
    .bind(payload.capacity_bytes)
    .bind(payload.used_bytes)
    .bind(payload.active_uploads)
    .bind(payload.active_downloads)
    .bind(payload.active_replications)
    .bind(payload.avg_rtt_ms)
    .bind(payload.p95_rtt_ms)
    .bind(payload.packet_loss)
    .bind(payload.timeout_rate)
    .bind(&node_id)
    .execute(&state.db)
    .await;

    if let Err(err) = result {
        tracing::error!("failed to update heartbeat: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    if current_status != new_status {
        audit::audit(
            "node_status_change",
            &node_id,
            Some(&node_id),
            "success",
            Some(&format!("{current_status} -> {new_status}")),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": new_status,
        })),
    )
        .into_response()
}

/// GET /api/storage/nodes — service discovery view
pub async fn list_nodes_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT id, endpoint, zone, status, capacity_bytes, used_bytes,
                active_uploads, active_downloads, active_replications,
                avg_rtt_ms, p95_rtt_ms, packet_loss, timeout_rate,
                heartbeat_success_rate, features, last_heartbeat_at
         FROM storage_nodes ORDER BY id",
    )
    .fetch_all(&state.db)
    .await;

    let nodes: Vec<serde_json::Value> = match rows {
        Ok(rows) => rows
            .iter()
            .map(|row| {
                let last_hb: Option<String> = row.get("last_heartbeat_at");
                let live = last_hb
                    .as_deref()
                    .and_then(|hb| {
                        chrono::NaiveDateTime::parse_from_str(hb, "%Y-%m-%d %H:%M:%S").ok()
                    })
                    .map(|dt| {
                        let elapsed = chrono::Utc::now().naive_utc().signed_duration_since(dt);
                        elapsed.num_seconds() <= state.node_config.heartbeat_ttl_secs as i64
                    })
                    .unwrap_or(false);

                let cap: i64 = row.get("capacity_bytes");
                let used: i64 = row.get("used_bytes");

                serde_json::json!({
                    "id": row.get::<String, _>("id"),
                    "endpoint": row.get::<String, _>("endpoint"),
                    "zone": row.get::<Option<String>, _>("zone"),
                    "status": row.get::<String, _>("status"),
                    "live": live,
                    "free_bytes": cap - used,
                    "active_uploads": row.get::<i64, _>("active_uploads"),
                    "active_downloads": row.get::<i64, _>("active_downloads"),
                    "active_replications": row.get::<i64, _>("active_replications"),
                    "avg_rtt_ms": row.get::<Option<i64>, _>("avg_rtt_ms"),
                    "p95_rtt_ms": row.get::<Option<i64>, _>("p95_rtt_ms"),
                    "packet_loss": row.get::<Option<f64>, _>("packet_loss"),
                    "timeout_rate": row.get::<Option<f64>, _>("timeout_rate"),
                    "features": row.get::<Option<String>, _>("features"),
                    "last_heartbeat_at": last_hb,
                })
            })
            .collect(),
        Err(err) => {
            tracing::error!("failed to list nodes: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    (StatusCode::OK, Json(serde_json::json!({ "nodes": nodes }))).into_response()
}

// ---------------------------------------------------------------------------
// Storage Node client-side (storage mode): registration + heartbeat loop
// ---------------------------------------------------------------------------

/// Spawn a background task that registers this node with the control plane
/// and sends periodic heartbeats.
#[allow(clippy::too_many_arguments)]
pub fn spawn_heartbeat_loop(
    api_endpoint: String,
    node_id: String,
    node_name: String,
    node_endpoint: String,
    zone: Option<String>,
    capacity_bytes: i64,
    internal_token: Option<String>,
    heartbeat_interval_secs: u64,
) {
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();

        // Registration with exponential backoff
        let mut retry_delay = std::time::Duration::from_secs(1);
        loop {
            let mut req = client.post(format!(
                "{}/api/storage/nodes/register",
                api_endpoint.trim_end_matches('/')
            ));
            if let Some(ref token) = internal_token {
                req = req.header("Authorization", format!("Bearer {token}"));
            }
            req = req.json(&serde_json::json!({
                "node_id": node_id,
                "name": node_name,
                "endpoint": node_endpoint,
                "zone": zone,
                "capacity_bytes": capacity_bytes,
                "storage_version": env!("CARGO_PKG_VERSION"),
                "features": ["chunk-read", "chunk-write", "checksum", "replication"],
            }));

            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::info!("storage node registered with control plane");
                    break;
                }
                Ok(resp) => {
                    tracing::warn!(
                        "registration failed: {} {}",
                        resp.status(),
                        resp.text().await.unwrap_or_default()
                    );
                }
                Err(err) => {
                    tracing::warn!("registration request failed: {err}");
                }
            }

            tracing::info!("retrying registration in {:?}", retry_delay);
            tokio::time::sleep(retry_delay).await;
            retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(60));
        }

        // Heartbeat loop
        let interval = std::time::Duration::from_secs(heartbeat_interval_secs);
        loop {
            tokio::time::sleep(interval).await;

            let mut req = client.post(format!(
                "{}/api/storage/nodes/{}/heartbeat",
                api_endpoint.trim_end_matches('/'),
                node_id,
            ));
            if let Some(ref token) = internal_token {
                req = req.header("Authorization", format!("Bearer {token}"));
            }

            // Collect local metrics
            let disk_stats = get_disk_stats();
            req = req.json(&serde_json::json!({
                "capacity_bytes": disk_stats.capacity,
                "used_bytes": disk_stats.used,
                "active_uploads": 0,
                "active_downloads": 0,
                "active_replications": 0,
                "disk_ok": true,
            }));

            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!("heartbeat sent successfully");
                }
                Ok(resp) => {
                    tracing::warn!(
                        "heartbeat failed: {} {}",
                        resp.status(),
                        resp.text().await.unwrap_or_default()
                    );
                }
                Err(err) => {
                    tracing::warn!("heartbeat request failed: {err}");
                }
            }
        }
    });
}

struct DiskStats {
    capacity: i64,
    used: i64,
}

fn get_disk_stats() -> DiskStats {
    // Simplified: report system-level disk info using sysinfo
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    if let Some(disk) = disks.list().first() {
        DiskStats {
            capacity: disk.total_space() as i64,
            used: (disk.total_space() - disk.available_space()) as i64,
        }
    } else {
        DiskStats {
            capacity: 0,
            used: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// SSRF prevention
// ---------------------------------------------------------------------------

fn validate_endpoint(endpoint: &str) -> Result<(), String> {
    let parsed = url::Url::parse(endpoint)
        .map_err(|_| "invalid endpoint URL".to_string())?;

    match parsed.scheme() {
        "https" => {}
        "http" => {
            // Allow HTTP only for development (could be further restricted)
            tracing::warn!("node endpoint uses HTTP (insecure): {endpoint}");
        }
        other => return Err(format!("unsupported scheme: {other}")),
    }

    let host = parsed.host_str().unwrap_or_default();
    if host.is_empty() {
        return Err("endpoint host is empty".to_string());
    }

    // Block loopback
    if host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host.starts_with("127.")
    {
        return Err("loopback addresses not allowed for node endpoints".to_string());
    }

    // Block link-local
    if host.starts_with("169.254.") || host.starts_with("fe80:") {
        return Err("link-local addresses not allowed".to_string());
    }

    // Block cloud metadata
    if host == "169.254.169.254" || host == "metadata.google.internal" {
        return Err("cloud metadata service addresses not allowed".to_string());
    }

    // Block userinfo in URL
    if parsed.username() != "" || parsed.password().is_some() {
        return Err("credentials in endpoint URL not allowed".to_string());
    }

    Ok(())
}
