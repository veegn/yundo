use clap::Parser;
use lru::LruCache;
use precision_proxy::{
    app, cache,
    common::{
        cleanup_temp_files, initialize_cache_dir, initialize_database, parse_socket_addr, AppState,
        Args,
    },
    constants::{
        MAX_WEB_COOKIE_SESSIONS, PROXY_CLIENT_TIMEOUT_SECS, WEB_PROXY_CLIENT_TIMEOUT_SECS,
    },
    history::spawn_history_cleanup_task,
    metrics::install_metrics_recorder,
    state::CacheUsageTracker,
};
use reqwest::Client;
use std::{num::NonZeroUsize, sync::Arc, time::Duration};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    initialize_cache_dir(&args.cache_dir).await;
    cleanup_temp_files(&args.cache_dir).await;

    let db = initialize_database(&args.cache_dir).await;
    let max_cache_size = args.cache_size;
    let max_file_size = args.max_file_size;

    tracing::info!("cache_dir = {}", args.cache_dir.display());
    tracing::info!("frontend_dist = {}", args.frontend_dist.display());
    tracing::info!("max_cache_size = {}", max_cache_size);
    tracing::info!("max_file_size = {}", max_file_size);
    tracing::info!("base_path = {}", args.base_path);
    tracing::info!("rate_limit_per_minute = {}", args.rate_limit_per_minute);

    if args.api_key.is_some() {
        tracing::info!("API key authentication enabled");
    }

    let metrics_handle = match install_metrics_recorder() {
        Ok(handle) => {
            tracing::info!("Prometheus metrics enabled at /metrics");
            Some(handle)
        }
        Err(err) => {
            tracing::warn!("failed to install Prometheus metrics recorder: {err}");
            None
        }
    };

    let client = Client::builder()
        .timeout(Duration::from_secs(PROXY_CLIENT_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .expect("failed to build HTTP client");

    let web_client = Client::builder()
        .timeout(Duration::from_secs(WEB_PROXY_CLIENT_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build web proxy HTTP client");

    let cache_usage = Arc::new(CacheUsageTracker::new());
    let shutdown_token = tokio_util::sync::CancellationToken::new();

    let state = Arc::new(AppState {
        client,
        web_client,
        cache_dir: args.cache_dir.clone(),
        max_cache_size,
        max_file_size,
        filebox_size: args.filebox_size,
        db: db.clone(),
        frontend_dist: args.frontend_dist.clone(),
        base_path: args.base_path.clone(),
        web_cookies: tokio::sync::Mutex::new(LruCache::new(
            NonZeroUsize::new(MAX_WEB_COOKIE_SESSIONS).unwrap(),
        )),
        cache_usage: cache_usage.clone(),
        api_key: args.api_key.clone(),
        metrics_handle,
        shutdown_token: shutdown_token.clone(),
    });

    // Initialize cache usage counter
    let initial_usage = cache::calculate_actual_usage(&state.cache_dir, &state.db).await;
    state.cache_usage.set(initial_usage);
    tracing::info!("Initial cache usage: {} bytes", initial_usage);

    // Spawn background tasks
    cache::spawn_cache_eviction_task(state.clone());
    precision_proxy::filebox::spawn_filebox_cleanup_task(state.clone());
    spawn_history_cleanup_task(db, shutdown_token.clone());

    let app = app::build_router(state, args.frontend_dist.clone());
    let addr = parse_socket_addr(&args.host, args.port);
    tracing::info!("listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");

    // Graceful shutdown handler
    let shutdown_signal = async move {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install CTRL+C signal handler");
        tracing::info!("Shutdown signal received, stopping gracefully...");
        shutdown_token.cancel();
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .expect("failed to serve application");

    tracing::info!("Server stopped");
}
