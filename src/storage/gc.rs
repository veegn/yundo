use crate::common::AppState;
use sqlx::Row;
use std::sync::Arc;

/// Spawn the background GC worker that processes pending `storage_gc_tasks`.
pub fn spawn_gc_worker(state: Arc<AppState>) {
    let worker_id = format!("gc-{}", state.node_config.node_id);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            if let Err(err) = run_gc_cycle(&state, &worker_id).await {
                tracing::error!("gc worker cycle error: {err}");
            }
        }
    });
}

async fn run_gc_cycle(state: &AppState, worker_id: &str) -> anyhow::Result<()> {
    // Claim up to 10 pending/failed tasks whose retry time has arrived
    let now_str = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let lock_until =
        (chrono::Utc::now() + chrono::Duration::minutes(5))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

    let tasks = sqlx::query(
        "UPDATE storage_gc_tasks
         SET locked_by = ?, locked_until = ?, updated_at = CURRENT_TIMESTAMP
         WHERE id IN (
           SELECT id FROM storage_gc_tasks
           WHERE status IN ('pending', 'failed')
             AND next_retry_at <= ?
             AND (locked_until IS NULL OR locked_until <= ?)
           ORDER BY created_at ASC
           LIMIT 10
         )
         RETURNING id, node_id, object_key, retry_count, max_retry",
    )
    .bind(worker_id)
    .bind(&lock_until)
    .bind(&now_str)
    .bind(&now_str)
    .fetch_all(&state.db)
    .await?;

    for task in &tasks {
        let task_id: String = task.get("id");
        let node_id: String = task.get("node_id");
        let object_key: String = task.get("object_key");
        let retry_count: i64 = task.get("retry_count");
        let max_retry: i64 = task.get("max_retry");

        tracing::debug!("gc: processing task {task_id}, node={node_id}, key={object_key}");

        let delete_result = if node_id == state.node_config.node_id {
            // Local delete
            state.storage_backend.delete_chunk(&object_key).await
        } else if let Some(ref token) = state.internal_token {
            // Remote delete via storage client
            let client = super::client::StorageNodeClient::new();
            let endpoint = get_node_endpoint(&state.db, &node_id).await;
            match endpoint {
                Some(ep) => client
                    .delete_chunk(&ep, &object_key, token)
                    .await
                    .map_err(|e| super::StorageError::Io(std::io::Error::other(e.to_string()))),
                None => {
                    tracing::warn!("gc: node {node_id} not found, marking task succeeded (orphan)");
                    mark_task_status(&state.db, &task_id, "succeeded", None).await;
                    continue;
                }
            }
        } else {
            // No token configured, can't reach remote node
            tracing::warn!(
                "gc: no internal token, cannot delete chunk on remote node {node_id}"
            );
            mark_task_failed(&state.db, &task_id, retry_count, max_retry, "no internal token")
                .await;
            continue;
        };

        match delete_result {
            Ok(()) => {
                tracing::debug!("gc: task {task_id} succeeded");
                mark_task_status(&state.db, &task_id, "succeeded", None).await;
                // Also clean up chunk_replicas entry
                let _ = sqlx::query(
                    "DELETE FROM chunk_replicas WHERE object_key = ? AND node_id = ?",
                )
                .bind(&object_key)
                .bind(&node_id)
                .execute(&state.db)
                .await;
            }
            Err(err) => {
                let err_str = err.to_string();
                tracing::warn!("gc: task {task_id} failed: {err_str}");
                mark_task_failed(&state.db, &task_id, retry_count, max_retry, &err_str)
                    .await;
            }
        }
    }

    Ok(())
}

async fn mark_task_status(db: &sqlx::SqlitePool, task_id: &str, status: &str, error: Option<&str>) {
    let _ = sqlx::query(
        "UPDATE storage_gc_tasks SET status = ?, last_error = ?, locked_by = NULL, locked_until = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(status)
    .bind(error)
    .bind(task_id)
    .execute(db)
    .await;
}

async fn mark_task_failed(
    db: &sqlx::SqlitePool,
    task_id: &str,
    retry_count: i64,
    max_retry: i64,
    error: &str,
) {
    let new_count = retry_count + 1;
    let new_status = if new_count >= max_retry {
        "abandoned"
    } else {
        "failed"
    };

    // Exponential backoff: 2^retry_count seconds, max 1 hour
    let delay_secs = (2_i64.pow(new_count.min(12) as u32)).min(3600);
    let next_retry = (chrono::Utc::now() + chrono::Duration::seconds(delay_secs))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    if new_status == "abandoned" {
        tracing::error!(
            "gc: task {task_id} abandoned after {new_count} retries: {error}"
        );
    }

    let _ = sqlx::query(
        "UPDATE storage_gc_tasks SET
            status = ?, retry_count = ?, next_retry_at = ?, last_error = ?,
            locked_by = NULL, locked_until = NULL, updated_at = CURRENT_TIMESTAMP
         WHERE id = ?",
    )
    .bind(new_status)
    .bind(new_count)
    .bind(&next_retry)
    .bind(error)
    .bind(task_id)
    .execute(db)
    .await;
}

async fn get_node_endpoint(db: &sqlx::SqlitePool, node_id: &str) -> Option<String> {
    sqlx::query("SELECT endpoint FROM storage_nodes WHERE id = ?")
        .bind(node_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .map(|row| row.get("endpoint"))
}

/// Helper: create a GC task for a chunk replica.
pub async fn create_gc_task(
    db: &sqlx::SqlitePool,
    file_id: Option<&str>,
    chunk_id: Option<&str>,
    node_id: &str,
    object_key: &str,
) {
    let task_id = format!("gc-{}", create_task_id());
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO storage_gc_tasks (id, file_id, chunk_id, node_id, object_key)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&task_id)
    .bind(file_id)
    .bind(chunk_id)
    .bind(node_id)
    .bind(object_key)
    .execute(db)
    .await;
}

pub fn create_task_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let random: u64 = nanos as u64 ^ (nanos >> 64) as u64;
    format!("{:016x}{:016x}", nanos as u64, random)
}
