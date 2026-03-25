use crate::common::{AppState, HistoryItem};
use axum::{extract::State, Json};
use sqlx::{Row, SqlitePool};
use std::sync::Arc;

pub(crate) fn spawn_history_cleanup_task(db: SqlitePool) {
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

pub(crate) async fn history_handler(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<HistoryItem>> {
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

pub(crate) async fn record_download(db: SqlitePool, url: String, file_name: String, file_size: i64) {
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
