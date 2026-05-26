use crate::common::AppState;
use crate::storage::audit;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

/// GET /api/admin/nodes — Detailed node list with metrics.
pub async fn list_nodes_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT * FROM storage_nodes ORDER BY id",
    )
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => {
            let nodes: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "id": row.get::<String, _>("id"),
                        "name": row.get::<String, _>("name"),
                        "endpoint": row.get::<String, _>("endpoint"),
                        "zone": row.get::<Option<String>, _>("zone"),
                        "status": row.get::<String, _>("status"),
                        "capacity_bytes": row.get::<i64, _>("capacity_bytes"),
                        "used_bytes": row.get::<i64, _>("used_bytes"),
                        "active_uploads": row.get::<i64, _>("active_uploads"),
                        "active_downloads": row.get::<i64, _>("active_downloads"),
                        "active_replications": row.get::<i64, _>("active_replications"),
                        "avg_rtt_ms": row.get::<Option<i64>, _>("avg_rtt_ms"),
                        "p95_rtt_ms": row.get::<Option<i64>, _>("p95_rtt_ms"),
                        "packet_loss": row.get::<Option<f64>, _>("packet_loss"),
                        "timeout_rate": row.get::<Option<f64>, _>("timeout_rate"),
                        "heartbeat_success_rate": row.get::<Option<f64>, _>("heartbeat_success_rate"),
                        "features": row.get::<Option<String>, _>("features"),
                        "storage_version": row.get::<Option<String>, _>("storage_version"),
                        "public_download": row.get::<i32, _>("public_download"),
                        "last_heartbeat_at": row.get::<Option<String>, _>("last_heartbeat_at"),
                        "created_at": row.get::<String, _>("created_at"),
                        "updated_at": row.get::<String, _>("updated_at"),
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "nodes": nodes }))).into_response()
        }
        Err(err) => {
            tracing::error!("admin list_nodes failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct SetNodeStatusPayload {
    pub status: String,
}

/// POST /api/admin/nodes/:id/status — Set node status (readonly/draining/offline).
pub async fn set_node_status_handler(
    State(state): State<Arc<AppState>>,
    Path(node_id): Path<String>,
    Json(payload): Json<SetNodeStatusPayload>,
) -> impl IntoResponse {
    let valid_statuses = [
        "registered", "active", "degraded", "readonly", "draining", "offline",
    ];
    if !valid_statuses.contains(&payload.status.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            format!("invalid status; must be one of: {}", valid_statuses.join(", ")),
        )
            .into_response();
    }

    // Check node exists
    let existing = sqlx::query("SELECT status FROM storage_nodes WHERE id = ?")
        .bind(&node_id)
        .fetch_optional(&state.db)
        .await;

    let old_status: String = match existing {
        Ok(Some(row)) => row.get("status"),
        Ok(None) => return (StatusCode::NOT_FOUND, "node not found").into_response(),
        Err(err) => {
            tracing::error!("admin set_node_status failed: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    // If draining, start the drain process
    if payload.status == "draining" {
        match super::repair::drain_node(&state.db, &node_id).await {
            Ok(task_count) => {
                tracing::info!(
                    "admin: draining node {node_id}, created {task_count} migration tasks"
                );
            }
            Err(err) => {
                tracing::error!("admin: drain failed: {err}");
                return (StatusCode::INTERNAL_SERVER_ERROR, err).into_response();
            }
        }
    }

    // Update status
    let _ = sqlx::query(
        "UPDATE storage_nodes SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(&payload.status)
    .bind(&node_id)
    .execute(&state.db)
    .await;

    audit::audit(
        "admin_set_node_status",
        "admin",
        Some(&node_id),
        "success",
        Some(&format!("{old_status} -> {}", payload.status)),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "node_id": node_id,
            "old_status": old_status,
            "new_status": payload.status,
        })),
    )
        .into_response()
}

/// GET /api/admin/repair-tasks — List repair tasks.
pub async fn list_repair_tasks_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT id, file_id, chunk_id, source_node_id, target_node_id,
                reason, status, priority, retry_count, next_retry_at,
                locked_by, last_error, created_at, updated_at
         FROM replica_repair_tasks
         ORDER BY priority ASC, created_at DESC
         LIMIT 100",
    )
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => {
            let tasks: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "id": row.get::<String, _>("id"),
                        "file_id": row.get::<Option<String>, _>("file_id"),
                        "chunk_id": row.get::<String, _>("chunk_id"),
                        "source_node_id": row.get::<Option<String>, _>("source_node_id"),
                        "target_node_id": row.get::<Option<String>, _>("target_node_id"),
                        "reason": row.get::<String, _>("reason"),
                        "status": row.get::<String, _>("status"),
                        "priority": row.get::<i64, _>("priority"),
                        "retry_count": row.get::<i64, _>("retry_count"),
                        "next_retry_at": row.get::<Option<String>, _>("next_retry_at"),
                        "locked_by": row.get::<Option<String>, _>("locked_by"),
                        "last_error": row.get::<Option<String>, _>("last_error"),
                        "created_at": row.get::<String, _>("created_at"),
                        "updated_at": row.get::<String, _>("updated_at"),
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "tasks": tasks }))).into_response()
        }
        Err(err) => {
            tracing::error!("admin list_repair_tasks failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// GET /api/admin/gc-tasks — List GC tasks.
pub async fn list_gc_tasks_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT id, file_id, chunk_id, node_id, object_key,
                status, retry_count, max_retry, next_retry_at,
                locked_by, last_error, created_at, updated_at
         FROM storage_gc_tasks
         ORDER BY created_at DESC
         LIMIT 100",
    )
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => {
            let tasks: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "id": row.get::<String, _>("id"),
                        "file_id": row.get::<Option<String>, _>("file_id"),
                        "chunk_id": row.get::<Option<String>, _>("chunk_id"),
                        "node_id": row.get::<String, _>("node_id"),
                        "object_key": row.get::<String, _>("object_key"),
                        "status": row.get::<String, _>("status"),
                        "retry_count": row.get::<i64, _>("retry_count"),
                        "max_retry": row.get::<i64, _>("max_retry"),
                        "next_retry_at": row.get::<Option<String>, _>("next_retry_at"),
                        "locked_by": row.get::<Option<String>, _>("locked_by"),
                        "last_error": row.get::<Option<String>, _>("last_error"),
                        "created_at": row.get::<String, _>("created_at"),
                        "updated_at": row.get::<String, _>("updated_at"),
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "tasks": tasks }))).into_response()
        }
        Err(err) => {
            tracing::error!("admin list_gc_tasks failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// POST /api/admin/repair-tasks/:id/retry — Manually retry a repair task.
pub async fn retry_repair_task_handler(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let result = sqlx::query(
        "UPDATE replica_repair_tasks SET
            status = 'pending', next_retry_at = CURRENT_TIMESTAMP,
            locked_by = NULL, locked_until = NULL, updated_at = CURRENT_TIMESTAMP
         WHERE id = ? AND status IN ('failed', 'abandoned')",
    )
    .bind(&task_id)
    .execute(&state.db)
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            (StatusCode::OK, Json(serde_json::json!({"retried": true}))).into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, "task not found or not retryable").into_response(),
        Err(err) => {
            tracing::error!("admin retry_repair_task failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// POST /api/admin/reconcile — Manually trigger reconciliation.
pub async fn trigger_reconcile_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // Just re-use the same reconcile logic from the repair worker
    // Note: this is a simplified trigger; the actual reconcile runs in background
    let under_replicated = sqlx::query(
        "SELECT fc.id as chunk_id, f.id as file_id, f.replication_factor,
                COUNT(cr.node_id) as replica_count
         FROM files f
         JOIN file_chunks fc ON fc.file_id = f.id
         LEFT JOIN chunk_replicas cr ON cr.chunk_id = fc.id AND cr.status = 'ready'
         WHERE f.status IN ('ready', 'partial_ready', 'repair_needed')
           AND (f.expires_at IS NULL OR f.expires_at >= datetime('now'))
         GROUP BY fc.id
         HAVING replica_count < f.replication_factor
         LIMIT 200",
    )
    .fetch_all(&state.db)
    .await;

    match under_replicated {
        Ok(rows) => {
            let count = rows.len();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "under_replicated_chunks": count,
                    "message": "reconciliation tasks will be processed by the repair worker"
                })),
            )
                .into_response()
        }
        Err(err) => {
            tracing::error!("admin reconcile failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}
