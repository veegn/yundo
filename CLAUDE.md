# CLAUDE.md

This file is the working guide for coding agents in this repository. Keep it aligned with the implementation; `README.md` is the user-facing operational documentation.

## Project

Yundo is a self-hosted HTTP/HTTPS download proxy and temporary FileBox service.

- Backend: Rust 2021, Axum, Tokio, Reqwest, SQLx/SQLite
- Frontend: React 19, Vite, TypeScript, React Router
- Runtime: the Rust binary serves the API and `frontend/dist` when it exists
- Storage: one local cache directory containing SQLite data, proxy cache files, FileBox files, and temporary upload chunks

The Cargo package is named `precision-proxy`, while the product is named Yundo.

## Commands

From the repository root:

```bash
npm install
npm run dev                 # frontend development server
npm run build               # build frontend/dist
npm run lint                # TypeScript check
cargo build
cargo fmt --check
cargo test
cargo run -- --cache-size 1GiB
```

The backend refuses to start without the required `--cache-size` argument. For a release binary:

```bash
npm run build
cargo build --release
./target/release/precision-proxy --cache-size 1GiB
```

## Backend map

- `src/main.rs`: CLI parsing, directory/database initialization, metrics setup, background tasks, listener, and graceful shutdown.
- `src/app.rs`: Axum route composition, optional frontend serving, base-path handling, and API-key middleware placement.
- `src/config.rs`: CLI flags, cache-size parsing, socket address parsing, and base-path normalization.
- `src/state.rs`: `AppState`, SQLite schema initialization, cache usage tracking, and startup temporary-file cleanup.
- `src/proxy.rs`: `GET`/`HEAD /api/proxy`, upstream requests, response header filtering, filename resolution, streaming, and full-response caching.
- `src/filebox.rs`: FileBox listing, direct uploads, chunk uploads/merges, remote transfer, range downloads, deletion, and expiry cleanup.
- `src/filebox_utils.rs`: atomic space reservation, release, upload-size checks, and upload-ID validation.
- `src/cache.rs`: proxy cache reads, usage calculation, and oldest-first eviction of proxy cache files.
- `src/history/`: download history storage, seven-day event cleanup, ranking, and `/api/recent` response conversion.
- `src/metrics.rs`: Prometheus recorder setup, `/metrics`, and HTTP request counters/histograms.
- `src/middleware.rs`: optional API-key authentication for FileBox mutation routes.
- `src/ssrf.rs`: URL/IP checks and the production HTTP client builder. Its DNS resolver validates the exact addresses handed to the connector; its redirect policy validates every target. Environment proxies are disabled. Tests may explicitly override DNS for isolated upstream servers.
- `src/storage.rs`: staged files, quota reservations, publication, and cleanup on errors or cancellation.
- `src/headers.rs`: allowed upstream headers and content-disposition/filename helpers.
- `src/handlers.rs`: health, root fallback, and not-found responses.
- `src/common.rs`: compatibility re-export facade for shared types and helpers.

## Routes

Public routes:

- `GET`/`HEAD /api/proxy?url=...`
- `GET /api/recent`
- `GET /api/filebox/files`
- `GET /api/filebox/download/:id`
- `GET /healthz`
- `GET /metrics` (returns `503` if the Prometheus recorder was not installed)

Protected only when `--api-key` is configured:

- `POST /api/filebox/upload`
- `POST /api/filebox/upload-chunk`
- `POST /api/filebox/upload-complete`
- `POST /api/filebox/upload-abort`
- `POST /api/filebox/remote-upload`
- `DELETE /api/filebox/delete/:id`

Authentication accepts `Authorization: Bearer <key>`, a plain `Authorization` value, or `X-API-Key: <key>`. The bundled frontend offers an API key field when authentication is enabled and stores it in sessionStorage, scoped to the base path.

The frontend routes are `/`, `/filebox`, `/privacy`, and `/terms`. It uses the runtime base path injected by the backend and calls the API through that prefix.

## Important behavior

- `--cache-size` is the shared limit for proxy payloads, all FileBox payloads on disk (including expired/orphaned files), and temporary chunks/output. `--filebox-size` is still parsed for CLI compatibility but is not used as a separate quota.
- `--max-file-size` defaults to `0` (unlimited). Direct and remote uploads, individual chunks, and the total merged file check the configured limit. Chunk bodies are streamed and capped at 72 MiB; JSON bodies are capped at 64 KiB.
- Full successful proxy responses are cached using paired `.data` and `.meta` files. `Range` proxy requests bypass the cache. FileBox downloads support byte ranges and return `206`/`416` as appropriate.
- FileBox metadata is stored in `proxy.db`; files expire after seven days and are removed by a background task.
- Every payload write reserves quota atomically, including temporary output during merges. Merge requires space for source chunks plus the assembled file. Failed writes release quota after cleanup; duplicates return 409. Proxy cache failures leave the forwarded download running.
- Disk reconciliation includes expired/orphaned FileBox files and temporary chunks, and only runs when no writer holds the storage operation guard. Cache eviction removes oldest proxy cache files; it does not evict active FileBox files.
- Startup removes staged `*.tmp` files at the cache root and in filebox/filebox_tmp. Abandoned chunk directories are cleaned up by the FileBox background task.
- CORS is permissive (`CorsLayer::permissive()`). Any deployment that needs origin restrictions must add an explicit policy.

## Configuration examples

Development with a frontend dev server:

```bash
cargo run -- --cache-size 1GiB
npm run dev
```

Sub-path deployment:

```bash
cargo run -- --cache-size 1GiB --base-path /yundo
```

Authentication:

```bash
cargo run -- --cache-size 1GiB --api-key your-secret-key
```

Docker builds the frontend in a separate stage and runs the Rust binary as a non-root user. The image command must include `--cache-size`.

## Testing guidance

Integration tests in `tests/integration.rs` exercise proxy `HEAD`, full-response caching, range-cache bypass, base-path routing, and FileBox range downloads. Tests use temporary cache directories and a local upstream server.

When changing:

- routes or base-path handling, update route/integration tests;
- cache behavior, verify both full downloads and `Range` requests;
- FileBox uploads or quota accounting, test direct, chunked, remote, cleanup, and failure paths;
- SSRF checks, cover both hostname/IP-literal validation and any DNS-resolution behavior actually wired into request handlers;
- frontend API calls or routes, run `npm run lint` and `npm run build`.

Do not rely on historical optimization reports; they were removed because they described superseded modules and stale implementation status.
