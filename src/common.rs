/// `common` is kept as a thin re-export facade so that all existing import paths
/// (`use crate::common::AppState`, etc.) continue to work without modification.
/// The actual implementations live in the focused modules below.
pub use crate::config::{parse_cache_size, parse_socket_addr, Args};
pub use crate::handlers::{health_handler, not_found_handler, root_handler};
pub use crate::headers::{
    build_content_disposition, ensure_download_filename, extract_filename_from_url,
    is_forbidden_host, resolve_file_name, ALLOWED_HEADERS,
};
pub use crate::state::{
    initialize_cache_dir, initialize_database, AppState, CacheMeta, HistoryItem, ProxyQuery,
};
