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

/// Maximum size for web proxy text rewriting (10 MB)
pub const MAX_REWRITE_BYTES: usize = 10 * 1024 * 1024;

/// Default maximum file size (500 MB)
pub const DEFAULT_MAX_FILE_SIZE: u64 = 500 * 1024 * 1024;

/// Channel buffer size for streaming responses
pub const STREAM_CHANNEL_BUFFER: usize = 16;

/// Maximum number of web proxy cookie sessions to keep in memory
pub const MAX_WEB_COOKIE_SESSIONS: usize = 1000;

/// Default database connection pool size
pub const DB_CONNECTION_POOL_SIZE: u32 = 20;

/// HTTP client timeout for proxy requests (300 seconds)
pub const PROXY_CLIENT_TIMEOUT_SECS: u64 = 300;

/// HTTP client timeout for web proxy requests (60 seconds)
pub const WEB_PROXY_CLIENT_TIMEOUT_SECS: u64 = 60;

/// Maximum number of redirects to follow
pub const MAX_REDIRECTS: usize = 10;

/// Rate limit: requests per minute per IP
pub const RATE_LIMIT_PER_MINUTE: u64 = 60;

/// Rate limit: burst size
pub const RATE_LIMIT_BURST: u64 = 10;
