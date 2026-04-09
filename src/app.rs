use crate::{
    common::{health_handler, not_found_handler, root_handler, AppState},
    history::{history_handler, resource_detail_handler, resources_index_handler},
    proxy::{proxy_handler, proxy_head_handler},
    seo::{robots_txt_handler, sitemap_xml_handler},
};
use axum::{routing::{get, get_service}, Router};
use std::{path::PathBuf, sync::Arc};
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

pub fn build_router(state: Arc<AppState>, frontend_dist: PathBuf) -> Router {
    let api_router = Router::new()
        .route("/api/proxy", get(proxy_handler).head(proxy_head_handler))
        .route("/api/recent", get(history_handler))
        .route("/api/history", get(history_handler))
        .route("/healthz", get(health_handler))
        .route("/robots.txt", get(robots_txt_handler))
        .route("/sitemap.xml", get(sitemap_xml_handler))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    if frontend_dist.join("index.html").exists() {
        tracing::info!("serving frontend assets from {}", frontend_dist.display());
        let index_file = frontend_dist.join("index.html");
        api_router
            .route_service("/", get_service(ServeFile::new(index_file.clone())))
            .route_service("/proxydash", get_service(ServeFile::new(index_file.clone())))
            .route_service("/index.html", get_service(ServeFile::new(index_file)))
            .route("/downloads", get(resources_index_handler))
            .route("/downloads/:slug", get(resource_detail_handler))
            .nest_service("/assets", ServeDir::new(frontend_dist.join("assets")))
            .fallback(get(not_found_handler))
            .with_state(state)
    } else {
        tracing::warn!(
            "frontend dist missing at {}, only API routes will be available",
            frontend_dist.display()
        );
        api_router
            .route("/", get(root_handler))
            .route("/downloads", get(resources_index_handler))
            .route("/downloads/:slug", get(resource_detail_handler))
            .fallback(get(not_found_handler))
            .with_state(state)
    }
}
