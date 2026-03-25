use crate::{
    common::{health_handler, root_handler, AppState},
    history::history_handler,
    proxy::{proxy_handler, proxy_head_handler},
};
use axum::{routing::get, Router};
use std::{path::PathBuf, sync::Arc};
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

pub(crate) fn build_router(state: Arc<AppState>, frontend_dist: PathBuf) -> Router {
    let api_router = Router::new()
        .route("/api/proxy", get(proxy_handler).head(proxy_head_handler))
        .route("/api/recent", get(history_handler))
        .route("/api/history", get(history_handler))
        .route("/healthz", get(health_handler))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    if frontend_dist.join("index.html").exists() {
        tracing::info!("serving frontend assets from {}", frontend_dist.display());
        api_router.fallback_service(
            ServeDir::new(&frontend_dist)
                .not_found_service(ServeFile::new(frontend_dist.join("index.html"))),
        )
    } else {
        tracing::warn!(
            "frontend dist missing at {}, only API routes will be available",
            frontend_dist.display()
        );
        api_router.route("/", get(root_handler))
    }
}
