mod app;
mod cache;
mod common;
mod history;
mod proxy;

use clap::Parser;
use common::{initialize_cache_dir, initialize_database, parse_socket_addr, AppState, Args};
use history::spawn_history_cleanup_task;
use reqwest::Client;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    initialize_cache_dir(&args.cache_dir).await;
    let db = initialize_database(&args.cache_dir).await;
    let max_cache_size = args.cache_size;

    tracing::info!("cache_dir = {}", args.cache_dir.display());
    tracing::info!("frontend_dist = {}", args.frontend_dist.display());
    tracing::info!("max_cache_size = {}", max_cache_size);

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .expect("failed to build HTTP client");

    let state = Arc::new(AppState {
        client,
        cache_dir: args.cache_dir.clone(),
        max_cache_size,
        db: db.clone(),
    });

    cache::spawn_cache_eviction_task(state.clone());
    spawn_history_cleanup_task(db);

    let app = app::build_router(state, args.frontend_dist.clone());
    let addr = parse_socket_addr(&args.host, args.port);
    tracing::info!("listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");
    axum::serve(listener, app)
        .await
        .expect("failed to serve application");
}
