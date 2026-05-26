use clap::Parser;
use precision_proxy::{
    app, cache,
    common::{initialize_cache_dir, initialize_database, AppState, NodeConfig, Args},
    config::NodeMode,
    history::spawn_history_cleanup_task,
    storage::{self, LocalStorageBackend},
};
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
    tracing::info!("base_path = {}", args.base_path);
    tracing::info!("node_mode = {}", args.node_mode);
    tracing::info!("node_id = {}", args.node_id);

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .expect("failed to build HTTP client");

    let storage_backend = Arc::new(LocalStorageBackend::new(args.cache_dir.join("storage")));

    let node_config = NodeConfig {
        node_id: args.node_id.clone(),
        endpoint: args.node_endpoint.clone(),
        zone: args.node_zone.clone(),
        api_endpoint: args.api_endpoint.clone(),
        heartbeat_interval_secs: args.node_heartbeat_interval,
        heartbeat_ttl_secs: args.node_heartbeat_ttl,
        default_chunk_size: args.default_chunk_size as i64,
        default_replication_factor: args.default_replication_factor,
    };

    let state = Arc::new(AppState {
        client,
        cache_dir: args.cache_dir.clone(),
        max_cache_size,
        filebox_size: max_cache_size,
        db: db.clone(),
        frontend_dist: args.frontend_dist.clone(),
        base_path: args.base_path.clone(),
        storage_backend,
        node_mode: args.node_mode,
        node_config,
        internal_token: args.internal_token.clone(),
    });

    // --- Spawn background tasks based on mode ---

    // Cache eviction and cleanup (api | all)
    if matches!(args.node_mode, NodeMode::Api | NodeMode::All) {
        cache::spawn_cache_eviction_task(state.clone());
        precision_proxy::filebox::spawn_filebox_cleanup_task(state.clone());
        spawn_history_cleanup_task(db.clone());

        // GC and Repair workers (api | all)
        storage::gc::spawn_gc_worker(state.clone());
        storage::repair::spawn_repair_worker(state.clone());
    }

    // Self-register as storage node in 'all' mode
    if args.node_mode == NodeMode::All {
        // In 'all' mode, register self as a local node
        let _ = sqlx::query(
            "INSERT INTO storage_nodes (id, name, endpoint, zone, status, capacity_bytes, features, last_heartbeat_at)
             VALUES (?, ?, 'local', ?, 'active', ?, 'chunk-read,chunk-write,checksum,replication', CURRENT_TIMESTAMP)
             ON CONFLICT(id) DO UPDATE SET
                status = 'active',
                last_heartbeat_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(&args.node_id)
        .bind(&args.node_id)
        .bind(args.node_zone.as_deref())
        .bind(args.cache_size as i64)
        .execute(&db)
        .await;

        tracing::info!("registered local storage node: {}", args.node_id);

        // Start self-heartbeat refresh (keeps last_heartbeat_at current)
        let hb_db = db.clone();
        let hb_node_id = args.node_id.clone();
        let hb_interval = args.node_heartbeat_interval;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(hb_interval)).await;
                let _ = sqlx::query(
                    "UPDATE storage_nodes SET last_heartbeat_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                )
                .bind(&hb_node_id)
                .execute(&hb_db)
                .await;
            }
        });
    }

    // Storage mode: register with control plane and start heartbeat loop
    if args.node_mode == NodeMode::Storage {
        let api_ep = args
            .api_endpoint
            .clone()
            .expect("--api-endpoint is required in storage mode");
        let node_ep = args
            .node_endpoint
            .clone()
            .expect("--node-endpoint is required in storage mode");

        storage::registration::spawn_heartbeat_loop(
            api_ep,
            args.node_id.clone(),
            args.node_id.clone(),
            node_ep,
            args.node_zone.clone(),
            args.cache_size as i64,
            args.internal_token.clone(),
            args.node_heartbeat_interval,
        );
    }

    let app = app::build_router(state, args.frontend_dist.clone());
    let addr = precision_proxy::config::parse_socket_addr(&args.host, args.port);
    tracing::info!("listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");
    axum::serve(listener, app)
        .await
        .expect("failed to serve application");
}
