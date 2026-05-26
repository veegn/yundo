use crate::common::AppState;
use sqlx::Row;
use std::sync::Arc;

/// Spawn the background repair worker that processes pending `replica_repair_tasks`.
pub fn spawn_repair_worker(state: Arc<AppState>) {
    let worker_id = format!("repair-{}", state.node_config.node_id);
    let reconcile_state = state.clone();

    // Repair task processor
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            if let Err(err) = run_repair_cycle(&state, &worker_id).await {
                tracing::error!("repair worker cycle error: {err}");
            }
        }
    });

    // Reconciliation scanner (less frequent)
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(120)).await;
            if let Err(err) = run_reconcile(&reconcile_state).await {
                tracing::error!("reconcile cycle error: {err}");
            }
        }
    });
}

async fn run_repair_cycle(state: &AppState, worker_id: &str) -> anyhow::Result<()> {
    let now_str = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let lock_until = (chrono::Utc::now() + chrono::Duration::minutes(10))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    let tasks = sqlx::query(
        "UPDATE replica_repair_tasks
         SET locked_by = ?, locked_until = ?, updated_at = CURRENT_TIMESTAMP
         WHERE id IN (
           SELECT id FROM replica_repair_tasks
           WHERE status IN ('pending', 'failed')
             AND next_retry_at <= ?
             AND (locked_until IS NULL OR locked_until <= ?)
           ORDER BY priority ASC, created_at ASC
           LIMIT 5
         )
         RETURNING id, file_id, chunk_id, source_node_id, target_node_id, reason, retry_count",
    )
    .bind(worker_id)
    .bind(&lock_until)
    .bind(&now_str)
    .bind(&now_str)
    .fetch_all(&state.db)
    .await?;

    for task in &tasks {
        let task_id: String = task.get("id");
        let chunk_id: String = task.get("chunk_id");
        let _file_id: Option<String> = task.get("file_id");
        let source_node_id: Option<String> = task.get("source_node_id");
        let target_node_id: Option<String> = task.get("target_node_id");
        let retry_count: i64 = task.get("retry_count");
        let reason: String = task.get("reason");

        tracing::info!("repair: processing task {task_id}, chunk={chunk_id}, reason={reason}");

        if let Err(err) =
            execute_repair(state, &task_id, &chunk_id, source_node_id.as_deref(), target_node_id.as_deref())
                .await
        {
            tracing::warn!("repair: task {task_id} failed: {err}");
            mark_repair_failed(&state.db, &task_id, retry_count, &err).await;
        }
    }

    Ok(())
}

async fn execute_repair(
    state: &AppState,
    task_id: &str,
    chunk_id: &str,
    source_node_id: Option<&str>,
    target_node_id: Option<&str>,
) -> Result<(), String> {
    let token = state
        .internal_token
        .as_deref()
        .ok_or("no internal token configured")?;
    let client = super::client::StorageNodeClient::new();

    // Find a healthy source replica
    let source_replica = match source_node_id {
        Some(sn) => {
            sqlx::query(
                "SELECT node_id, object_key, sha256 FROM chunk_replicas
                 WHERE chunk_id = ? AND node_id = ? AND status = 'ready'",
            )
            .bind(chunk_id)
            .bind(sn)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
        }
        None => {
            sqlx::query(
                "SELECT node_id, object_key, sha256 FROM chunk_replicas
                 WHERE chunk_id = ? AND status = 'ready'
                 LIMIT 1",
            )
            .bind(chunk_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
        }
    };

    let source = source_replica.ok_or("no healthy source replica found")?;
    let src_node_id: String = source.get("node_id");
    let src_object_key: String = source.get("object_key");
    let expected_sha256: String = source.get("sha256");

    // Read chunk data from source
    let data = if src_node_id == state.node_config.node_id {
        // Local read
        let file = state
            .storage_backend
            .open_chunk(&src_object_key)
            .await
            .map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        use tokio::io::AsyncReadExt;
        tokio::io::BufReader::new(file)
            .read_to_end(&mut buf)
            .await
            .map_err(|e| e.to_string())?;
        bytes::Bytes::from(buf)
    } else {
        let src_endpoint = get_node_endpoint(&state.db, &src_node_id)
            .await
            .ok_or(format!("source node {src_node_id} endpoint not found"))?;
        client
            .get_chunk(&src_endpoint, &src_object_key, token)
            .await
            .map_err(|e| e.to_string())?
    };

    // Determine target node
    let target_id = match target_node_id {
        Some(tn) => tn.to_string(),
        None => {
            // Use scheduler to pick a target
            let discovery = super::discovery::ServiceDiscovery::new(
                state.db.clone(),
                5,
            );
            let writable = discovery
                .get_writable_nodes(state.node_config.heartbeat_ttl_secs)
                .await;
            // Filter out nodes that already have a replica
            let existing_nodes: Vec<String> = sqlx::query(
                "SELECT node_id FROM chunk_replicas WHERE chunk_id = ? AND status = 'ready'",
            )
            .bind(chunk_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
            .iter()
            .map(|r| r.get("node_id"))
            .collect();

            let candidates: Vec<_> = writable
                .into_iter()
                .filter(|n| !existing_nodes.contains(&n.id))
                .collect();

            let selected = super::scheduler::select_upload_nodes(
                &candidates,
                1,
                data.len() as i64,
            );
            selected
                .first()
                .map(|n| n.id.clone())
                .ok_or("no suitable target node found")?
        }
    };

    // Write chunk to target
    let target_object_key = src_object_key.clone(); // Same object key for replication
    let result = if target_id == state.node_config.node_id {
        state
            .storage_backend
            .put_chunk(&target_object_key, data, &expected_sha256)
            .await
            .map_err(|e| e.to_string())?
    } else {
        let target_endpoint = get_node_endpoint(&state.db, &target_id)
            .await
            .ok_or(format!("target node {target_id} endpoint not found"))?;
        client
            .put_chunk(
                &target_endpoint,
                &target_object_key,
                data,
                &expected_sha256,
                token,
                chunk_id, // file_id not available here
                0,
            )
            .await
            .map_err(|e| e.to_string())?
    };

    // Verify sha256
    if result.sha256 != expected_sha256 {
        return Err(format!(
            "sha256 mismatch after replication: expected {expected_sha256}, got {}",
            result.sha256
        ));
    }

    // Insert new chunk_replicas row
    sqlx::query(
        "INSERT OR REPLACE INTO chunk_replicas
         (chunk_id, node_id, object_key, size_bytes, sha256, status, verified_at)
         VALUES (?, ?, ?, ?, ?, 'ready', CURRENT_TIMESTAMP)",
    )
    .bind(chunk_id)
    .bind(&target_id)
    .bind(&target_object_key)
    .bind(result.size_bytes as i64)
    .bind(&result.sha256)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Mark repair task as succeeded
    let _ = sqlx::query(
        "UPDATE replica_repair_tasks SET status = 'succeeded', locked_by = NULL, locked_until = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(task_id)
    .execute(&state.db)
    .await;

    // Check if file is now fully replicated and update status
    if let Some(ref file_id) = sqlx::query("SELECT file_id FROM replica_repair_tasks WHERE id = ?")
        .bind(task_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.get::<Option<String>, _>("file_id"))
    {
        update_file_status_after_repair(&state.db, file_id).await;
    }

    tracing::info!("repair: task {task_id} succeeded, chunk {chunk_id} replicated to {target_id}");
    Ok(())
}

async fn update_file_status_after_repair(db: &sqlx::SqlitePool, file_id: &str) {
    // Check: all chunks have >= replication_factor ready replicas?
    let row = sqlx::query(
        "SELECT f.replication_factor, f.total_chunks,
                COUNT(DISTINCT fc.chunk_index) as chunks_ok
         FROM files f
         JOIN file_chunks fc ON fc.file_id = f.id
         WHERE f.id = ?
           AND (
               SELECT COUNT(*) FROM chunk_replicas cr
               WHERE cr.chunk_id = fc.id AND cr.status = 'ready'
           ) >= f.replication_factor
         GROUP BY f.id",
    )
    .bind(file_id)
    .fetch_optional(db)
    .await;

    if let Ok(Some(row)) = row {
        let total: i64 = row.get("total_chunks");
        let ok: i64 = row.get("chunks_ok");
        if ok >= total {
            let _ = sqlx::query(
                "UPDATE files SET status = 'ready', updated_at = CURRENT_TIMESTAMP WHERE id = ? AND status = 'partial_ready'",
            )
            .bind(file_id)
            .execute(db)
            .await;
            tracing::info!("repair: file {file_id} fully replicated, status -> ready");
        }
    }
}

async fn mark_repair_failed(db: &sqlx::SqlitePool, task_id: &str, retry_count: i64, error: &str) {
    let new_count = retry_count + 1;
    let delay_secs = (2_i64.pow(new_count.min(12) as u32)).min(3600);
    let next_retry = (chrono::Utc::now() + chrono::Duration::seconds(delay_secs))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    let new_status = if new_count >= 10 { "abandoned" } else { "failed" };
    if new_status == "abandoned" {
        tracing::error!("repair: task {task_id} abandoned after {new_count} retries");
    }

    let _ = sqlx::query(
        "UPDATE replica_repair_tasks SET
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

/// Reconciliation: Scan for under-replicated files and create repair tasks.
async fn run_reconcile(state: &AppState) -> anyhow::Result<()> {
    // Find chunks that have fewer ready replicas than the file's replication_factor
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
         LIMIT 50",
    )
    .fetch_all(&state.db)
    .await?;

    for row in &under_replicated {
        let chunk_id: String = row.get("chunk_id");
        let file_id: String = row.get("file_id");

        // Check no existing pending/failed repair task for this chunk
        let existing = sqlx::query(
            "SELECT id FROM replica_repair_tasks
             WHERE chunk_id = ? AND status IN ('pending', 'failed')
             LIMIT 1",
        )
        .bind(&chunk_id)
        .fetch_optional(&state.db)
        .await?;

        if existing.is_some() {
            continue; // Already has a pending task
        }

        let task_id = format!("repair-{}", super::gc::create_task_id());
        sqlx::query(
            "INSERT INTO replica_repair_tasks (id, file_id, chunk_id, reason, status, priority)
             VALUES (?, ?, ?, 'under_replicated', 'pending', 100)",
        )
        .bind(&task_id)
        .bind(&file_id)
        .bind(&chunk_id)
        .execute(&state.db)
        .await?;

        tracing::info!("reconcile: created repair task {task_id} for chunk {chunk_id}");
    }

    // Check for corrupt replicas
    let corrupt_replicas = sqlx::query(
        "SELECT cr.chunk_id, cr.node_id, cr.object_key
         FROM chunk_replicas cr
         WHERE cr.status = 'corrupt'
         LIMIT 20",
    )
    .fetch_all(&state.db)
    .await?;

    for row in &corrupt_replicas {
        let chunk_id: String = row.get("chunk_id");
        let node_id: String = row.get("node_id");
        let object_key: String = row.get("object_key");

        // Create GC task to delete the corrupt replica
        super::gc::create_gc_task(&state.db, None, Some(&chunk_id), &node_id, &object_key).await;

        // Mark the replica for deletion
        let _ = sqlx::query(
            "UPDATE chunk_replicas SET status = 'deleting', updated_at = CURRENT_TIMESTAMP WHERE chunk_id = ? AND node_id = ?",
        )
        .bind(&chunk_id)
        .bind(&node_id)
        .execute(&state.db)
        .await;
    }

    // Check for offline nodes and mark their replicas as needing repair
    let offline_nodes = sqlx::query(
        "SELECT id FROM storage_nodes
         WHERE status = 'offline'
           OR (last_heartbeat_at IS NOT NULL
               AND datetime(last_heartbeat_at, '+' || ? || ' seconds') < datetime('now'))",
    )
    .bind(state.node_config.heartbeat_ttl_secs as i64 * 3) // 3x TTL = truly gone
    .fetch_all(&state.db)
    .await?;

    for node_row in &offline_nodes {
        let node_id: String = node_row.get("id");
        // Find replicas on this node that should be re-replicated
        let replicas = sqlx::query(
            "SELECT cr.chunk_id FROM chunk_replicas cr
             WHERE cr.node_id = ? AND cr.status = 'ready'
             LIMIT 20",
        )
        .bind(&node_id)
        .fetch_all(&state.db)
        .await?;

        for rep in &replicas {
            let chunk_id: String = rep.get("chunk_id");
            // Check if repair task already exists
            let existing = sqlx::query(
                "SELECT id FROM replica_repair_tasks
                 WHERE chunk_id = ? AND status IN ('pending', 'failed')
                 LIMIT 1",
            )
            .bind(&chunk_id)
            .fetch_optional(&state.db)
            .await?;

            if existing.is_none() {
                let task_id = format!("repair-{}", super::gc::create_task_id());
                sqlx::query(
                    "INSERT INTO replica_repair_tasks (id, file_id, chunk_id, reason, status, priority)
                     VALUES (?, NULL, ?, 'node_offline', 'pending', 50)",
                )
                .bind(&task_id)
                .bind(&chunk_id)
                .execute(&state.db)
                .await?;
            }
        }
    }

    Ok(())
}

/// Process node draining: create repair tasks for all replicas on the draining node.
pub async fn drain_node(db: &sqlx::SqlitePool, node_id: &str) -> Result<i64, String> {
    let replicas = sqlx::query(
        "SELECT chunk_id FROM chunk_replicas WHERE node_id = ? AND status = 'ready'",
    )
    .bind(node_id)
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())?;

    let mut created = 0_i64;
    for rep in &replicas {
        let chunk_id: String = rep.get("chunk_id");
        let task_id = format!("drain-{}", super::gc::create_task_id());
        let result = sqlx::query(
            "INSERT OR IGNORE INTO replica_repair_tasks
             (id, chunk_id, source_node_id, reason, status, priority)
             VALUES (?, ?, ?, 'node_draining', 'pending', 10)",
        )
        .bind(&task_id)
        .bind(&chunk_id)
        .bind(node_id)
        .execute(db)
        .await;

        if let Ok(r) = result {
            if r.rows_affected() > 0 {
                created += 1;
            }
        }
    }

    // Mark node as draining
    let _ = sqlx::query(
        "UPDATE storage_nodes SET status = 'draining', updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(node_id)
    .execute(db)
    .await;

    tracing::info!("drain: created {created} repair tasks for node {node_id}");
    Ok(created)
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
