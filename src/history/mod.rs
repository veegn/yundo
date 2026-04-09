mod db;
mod routes;

// Data layer — re-exported for use by seo.rs and the integration tests.
pub use db::{
    build_history_slug, load_ranked_history_items, record_download,
    spawn_history_cleanup_task, to_history_item, RankedHistoryItem, SearchQuery,
};

// HTTP handlers — re-exported for use in app.rs.
pub use routes::{history_handler, resource_detail_handler, resources_index_handler};
