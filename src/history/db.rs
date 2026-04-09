use crate::common::HistoryItem;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

/// A history item with a computed popularity score used for ranking.
#[derive(Clone)]
pub struct RankedHistoryItem {
    pub slug: String,
    pub url: String,
    pub file_name: String,
    pub file_size: i64,
    pub last_download_at: String,
    pub count_7d: i64,
    pub score: f64,
}

/// Query parameters for the `/downloads` search endpoint.
#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

// ---------------------------------------------------------------------------
// Background task
// ---------------------------------------------------------------------------

pub fn spawn_history_cleanup_task(db: SqlitePool) {
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

// ---------------------------------------------------------------------------
// Write path
// ---------------------------------------------------------------------------

/// Records a download event and upserts the download_history row.
/// Spawned as a background task so it never blocks the response stream.
pub async fn record_download(
    db: SqlitePool,
    url: String,
    file_name: String,
    file_size: i64,
) {
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

// ---------------------------------------------------------------------------
// Read path
// ---------------------------------------------------------------------------

/// Loads up to 50 history items ranked by a time-decayed download count score.
/// An optional `query_str` filters results by file name (SQL LIKE).
pub async fn load_ranked_history_items(
    db: &SqlitePool,
    query_str: Option<&str>,
) -> Vec<RankedHistoryItem> {
    let query_pattern = query_str
        .map(|q| format!("%{q}%"))
        .unwrap_or_else(|| "%".to_string());

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
        FROM download_history h
        WHERE h.file_name LIKE ?",
    )
    .bind(query_pattern)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let mut items = rows
        .into_iter()
        .map(|row| {
            let url: String = row.get("url");
            let file_name: String = row.get("file_name");
            let count_7d: i64 = row.get("count_7d");
            let hours_since_last: f64 = row.get("hours_since_last");
            let score =
                ((count_7d as f64 + 1.0).powf(0.8)) / ((hours_since_last + 2.0).powf(1.5));

            RankedHistoryItem {
                slug: build_history_slug(&file_name, &url),
                url,
                file_name,
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
    items
}

/// Builds the stable URL slug for a history item.
pub fn build_history_slug(file_name: &str, url: &str) -> String {
    let base = slugify(file_name);
    let hash = short_hash(url);
    format!("{base}-{hash}")
}

/// Converts a `RankedHistoryItem` to the JSON-serialisable `HistoryItem`.
pub fn to_history_item(item: RankedHistoryItem) -> HistoryItem {
    HistoryItem {
        slug: item.slug,
        url: item.url,
        file_name: item.file_name,
        file_size: item.file_size,
        last_download_at: item.last_download_at,
        count_7d: item.count_7d,
        score: item.score,
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn slugify(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut last_was_dash = false;

    for ch in input.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            output.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            output.push('-');
            last_was_dash = true;
        }
    }

    let trimmed = output.trim_matches('-');
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.to_string()
    }
}

fn short_hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())[..8].to_string()
}
