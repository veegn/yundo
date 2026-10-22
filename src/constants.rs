/// Configuration constants for the application.
/// These values can be overridden via command-line arguments where applicable.
use std::time::Duration;

/// Default interval for cache eviction task (60 seconds)
pub const CACHE_EVICTION_INTERVAL: Duration = Duration::from_secs(60);

/// Default interval for filebox cleanup task (60 seconds)
pub const FILEBOX_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

/// Default interval for history cleanup task (3600 seconds = 1 hour)
pub const HISTORY_CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);

/// Default expiration time for filebox files (7 days)
pub const FILEBOX_EXPIRATION_DAYS: i64 = 7;

/// Maximum age for abandoned temporary chunk directories (24 hours)
pub const TEMP_CHUNK_MAX_AGE_SECS: u64 = 86400;

/// Default maximum file size (0 = unlimited)
pub const DEFAULT_MAX_FILE_SIZE: u64 = 0;

/// Channel buffer size for streaming responses
pub const STREAM_CHANNEL_BUFFER: usize = 16;

/// Default database connection pool size
pub const DB_CONNECTION_POOL_SIZE: u32 = 20;

/// HTTP client timeout for proxy requests (300 seconds)
pub const PROXY_CLIENT_TIMEOUT_SECS: u64 = 300;

/// Maximum number of redirects to follow
pub const MAX_REDIRECTS: usize = 10;

/// Match the bundled frontend's chunk size, enforced while streaming.
pub const MAX_UPLOAD_CHUNK_BYTES: u64 = 72 * 1024 * 1024;
