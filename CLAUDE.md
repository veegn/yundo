# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Yundo is a download proxy and history dashboard with three main features:
1. **Proxy downloads** - proxies HTTP/HTTPS files with HEAD detection, Range support, and disk caching
2. **FileBox** - file upload/download service with chunked uploads and expiration
3. **Web Proxy** - browse remote websites through the proxy with cookie/session management

Stack: Rust (Axum) backend, React (Vite) frontend, SQLite storage. The Rust server serves both API and static frontend assets.

## Build & Run Commands

### Development

Frontend only (hot reload):
```bash
npm run dev --workspace=frontend
```

Backend only (API routes):
```bash
cargo run -- --cache-size 1GiB
```

Full stack (build frontend + run backend):
```bash
npm run build
cargo run -- --cache-size 1GiB
```

With authentication:
```bash
cargo run -- --cache-size 1GiB --api-key your-secret-key
```

### Testing

Run all tests:
```bash
cargo test
```

Run specific test:
```bash
cargo test test_name
```

Run with output:
```bash
cargo test -- --nocapture
```

### Linting

Frontend TypeScript check:
```bash
npm run lint --workspace=frontend
```

Rust format check:
```bash
cargo fmt --check
```

### Production Build

```bash
npm run build
cargo build --release
./target/release/precision-proxy --cache-size 1GiB --max-file-size 500MB
```

## Architecture

### Backend Structure

The backend is organized into focused modules:

- **`app.rs`** - Router configuration with authentication middleware. Supports deployment behind reverse proxies with `X-Forwarded-Prefix` header. Injects runtime base path into SPA index.html.
- **`proxy.rs`** - Core download proxy logic (`/api/proxy`). Handles HEAD requests, Range headers, SSRF protection, and cache coordination.
- **`web_proxy.rs`** - Web browsing proxy (`/browse`). Rewrites HTML/CSS/JS to route through proxy, manages cookies per target domain with LRU cache (max 1000 sessions).
- **`filebox.rs`** - File upload/download service with chunked upload support, automatic expiration cleanup, and atomic quota reservation.
- **`filebox_utils.rs`** - Quota management helpers (`try_reserve_space`, `release_space`) and validation functions.
- **`history/mod.rs`** - Download history tracking with 7-day hot score calculation.
- **`cache.rs`** - LRU cache eviction based on disk usage with atomic usage tracking. Runs background task to enforce `max_cache_size`.
- **`state.rs`** - Shared `AppState` with `CacheUsageTracker` for atomic usage counting, database schema definitions, and graceful shutdown support.
- **`ssrf.rs`** - Enhanced SSRF protection with DNS resolution checking to prevent DNS rebinding attacks.
- **`middleware.rs`** - Authentication middleware for API key validation.
- **`errors.rs`** - Unified error types using `thiserror`.
- **`constants.rs`** - Centralized configuration constants (intervals, timeouts, limits).
- **`headers.rs`** - HTTP header utilities for content-type detection and filename extraction.

### Frontend Structure

React SPA with client-side routing:

- **`App.tsx`** - Main router with base path support from `window.__YUNDO_BASE_PATH__`
- **`pages/ProxyDash.tsx`** - Download history dashboard with hot downloads
- **`pages/FileBox.tsx`** - File upload/download UI with chunked upload
- **`pages/WebProxy.tsx`** - Web browsing interface
- **`context/I18nContext.tsx`** - Internationalization (Chinese/English)

### Key Patterns

**Atomic Quota Management**: 
- `CacheUsageTracker` maintains atomic counter of cache usage
- `try_reserve_space()` atomically reserves space before upload
- `release_space()` releases on failure or completion
- Prevents concurrent upload race conditions
- Recalibrates actual usage every 5 minutes

**Enhanced SSRF Protection**:
- Two-layer validation: hostname check + DNS resolution
- Blocks private IPs: 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
- Blocks loopback: 127.0.0.0/8, ::1
- Blocks link-local: 169.254.0.0/16, fe80::/10
- Prevents DNS rebinding attacks

**Authentication**:
- Optional API key via `--api-key` flag
- Protects upload/delete operations
- Supports `Authorization: Bearer <key>` or `X-API-Key` header
- Public routes: download, proxy, browse, healthz

**Base Path Support**: 
- The app supports deployment at non-root paths (e.g., `/yundo`)
- Backend reads `X-Forwarded-Prefix` header and injects `window.__YUNDO_BASE_PATH__` into the SPA
- Frontend uses this for all routing and API calls

**Cache Flow**: 
1. Request arrives at `/api/proxy?url=...`
2. Check if cached (hash-based filename in `cache_dir`)
3. If cached and no Range header, serve from disk
4. Otherwise, fetch from upstream, stream to client, save to cache
5. Background task evicts oldest files when total size exceeds `max_cache_size`
6. Atomic counter tracks usage in real-time

**History Tracking**: 
- Every download creates a `download_events` record
- The `/api/recent` endpoint calculates a "hot score" based on 7-day download frequency and recency

**Web Proxy Rewriting**: 
- HTML/CSS/JS responses are rewritten to route asset requests through `/browse/{encoded_target_url}`
- Cookies are stored per target domain in LRU cache (max 1000 sessions)

**Graceful Shutdown**:
- All background tasks listen for `CancellationToken`
- CTRL+C triggers graceful shutdown
- Tasks complete current work before stopping

## Important Notes

- `--cache-size` is **required** - the app will not start without it
- Cache size supports units: `512MB`, `2GB`, `1GiB` (binary) or raw bytes
- `--max-file-size` limits individual file uploads (default: 500MB)
- SQLite database is created at `{cache_dir}/proxy.db`
- FileBox files expire after 7 days (configurable in `constants.rs`)
- SSRF protection blocks private IPs and performs DNS resolution checks
- The package name is `precision-proxy` but the project is called Yundo
- Temporary files are cleaned up on startup to prevent disk space leaks

## Configuration Flags

```bash
--cache-dir <PATH>              # Cache directory (default: ./cache)
--cache-size <SIZE>             # Total cache size (required, e.g., 1GiB)
--max-file-size <SIZE>          # Max single file size (default: 500MB)
--host <IP>                     # Bind address (default: 0.0.0.0)
--port <PORT>                   # Port (default: 8080)
--frontend-dist <PATH>          # Frontend build directory (default: ./frontend/dist)
--base-path <PATH>              # Base URL path (default: /)
--api-key <KEY>                 # API key for authentication (optional)
--rate-limit-per-minute <NUM>   # Rate limit per IP (default: 60, not yet enforced)
```

## Testing

Integration tests in `tests/integration.rs` spawn real HTTP servers and test end-to-end flows. Tests use `tempfile::TempDir` for isolated cache directories.

Unit tests exist in:
- `ssrf.rs` - SSRF protection validation
- `filebox_utils.rs` - Upload ID validation
- `web_proxy.rs` - HTML/CSS rewriting

When adding features:
- Add integration tests for new API endpoints
- Test base path handling if modifying routing
- Test cache eviction if changing cache logic
- Test quota reservation for upload operations
- Test SSRF protection for new proxy endpoints

## Recent Optimizations

The codebase has been optimized for:
- **Performance**: Atomic cache usage tracking (~100x faster than directory scanning)
- **Security**: Enhanced SSRF protection with DNS checking, API key authentication
- **Reliability**: Atomic quota reservation prevents race conditions
- **Maintainability**: Centralized constants, unified error handling
- **Resource Management**: LRU cookie cache, graceful shutdown, temp file cleanup

See `OPTIMIZATION_SUMMARY.md` for detailed changes.
