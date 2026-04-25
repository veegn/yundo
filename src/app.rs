use crate::{
    common::{health_handler, root_handler, AppState},
    history::{history_handler, resource_detail_handler, resources_index_handler},
    proxy::{proxy_handler, proxy_head_handler},
    seo::{prefix_path, robots_txt_handler, sitemap_xml_handler},
};
use axum::{
    extract::{OriginalUri, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect},
    routing::get,
    Router,
};
use std::{path::PathBuf, sync::Arc};
use tokio::fs;
use tower_http::{
    cors::CorsLayer,
    services::ServeDir,
    trace::TraceLayer,
};

pub fn build_router(state: Arc<AppState>, frontend_dist: PathBuf) -> Router {
    let mut inner_router = Router::new()
        .route("/api/proxy", get(proxy_handler).head(proxy_head_handler))
        .route("/api/recent", get(history_handler))
        .route("/api/history", get(history_handler))
        .route("/healthz", get(health_handler))
        .route("/robots.txt", get(robots_txt_handler))
        .route("/sitemap.xml", get(sitemap_xml_handler))
        .route("/downloads", get(resources_index_handler))
        .route("/downloads/:slug", get(resource_detail_handler))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    if frontend_dist.join("index.html").exists() {
        tracing::info!("serving frontend assets from {}", frontend_dist.display());
        inner_router = inner_router
            .route("/", get(spa_index_handler))
            .route("/proxydash", get(spa_index_handler))
            .route("/index.html", get(spa_index_handler))
            .nest_service("/assets", ServeDir::new(frontend_dist.join("assets")))
            .fallback(get(base_aware_not_found_handler));
    } else {
        tracing::warn!(
            "frontend dist missing at {}, only API routes will be available",
            frontend_dist.display()
        );
        inner_router = inner_router
            .route("/", get(root_handler))
            .fallback(get(base_aware_not_found_handler));
    }

    let inner_router = inner_router.with_state(state.clone());

    if state.base_path == "/" {
        inner_router
    } else {
        let redirect_target = state.base_path.clone();
        Router::new()
            .route("/", get(move || async move { Redirect::permanent(&redirect_target) }))
            .nest(&state.base_path, inner_router)
    }
}

async fn spa_index_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let index_path = state.frontend_dist.join("index.html");
    let Ok(template) = fs::read_to_string(&index_path).await else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Frontend index.html could not be read from the configured dist directory.",
        )
            .into_response();
    };

    let request_base_path = crate::seo::derive_external_base_path(&headers, &state.base_path);
    let injected = inject_runtime_base_path(&template, &request_base_path);

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(injected),
    )
        .into_response()
}

fn inject_runtime_base_path(template: &str, base_path: &str) -> String {
    let base_href = ensure_trailing_slash(base_path);
    let runtime = format!(
        r#"<base href="{base_href}">
<script>window.__YUNDO_BASE_PATH__ = "{base_path}";</script>"#
    );

    if template.contains("</head>") {
        template.replacen("</head>", &format!("{runtime}\n  </head>"), 1)
    } else {
        format!("{runtime}\n{template}")
    }
}

fn ensure_trailing_slash(path: &str) -> String {
    let prefixed = prefix_path("/", path);
    if prefixed.ends_with('/') {
        prefixed
    } else {
        format!("{prefixed}/")
    }
}

async fn base_aware_not_found_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> impl IntoResponse {
    let base_path = crate::seo::derive_external_base_path(&headers, &state.base_path);
    let home_path = prefix_path(&base_path, "/");
    let escaped_path = uri
        .path_and_query()
        .map(|value| html_escape(value.as_str()))
        .unwrap_or_else(|| "/".to_string());

    (
        StatusCode::NOT_FOUND,
        [("content-type", "text/html; charset=utf-8")],
        Html(format!(
            r#"<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>404 - 页面不存在</title>
    <style>
      body {{
        margin: 0;
        min-height: 100vh;
        display: grid;
        place-items: center;
        background: linear-gradient(180deg, #f7f9ff 0%, #eef4ff 100%);
        color: #171c22;
        font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }}
      main {{
        width: min(92vw, 560px);
        padding: 40px 32px;
        background: rgba(255, 255, 255, 0.92);
        border: 1px solid rgba(194, 198, 214, 0.5);
        border-radius: 24px;
        box-shadow: 0 24px 80px rgba(23, 28, 34, 0.08);
        text-align: center;
      }}
      h1 {{
        margin: 0 0 12px;
        font-size: 56px;
        line-height: 1;
        color: #0058bb;
      }}
      p {{
        margin: 0 0 14px;
        line-height: 1.7;
        color: #424753;
      }}
      code {{
        display: inline-block;
        max-width: 100%;
        overflow-wrap: anywhere;
        padding: 2px 8px;
        border-radius: 999px;
        background: #eef4ff;
        color: #0058bb;
      }}
      a {{
        color: #0058bb;
        font-weight: 600;
        text-decoration: none;
      }}
    </style>
  </head>
  <body>
    <main>
      <h1>404</h1>
      <p>你访问的页面不存在。</p>
      <p>当前路径：<code>{escaped_path}</code></p>
      <p><span id="countdown">5</span> 秒后将返回首页。</p>
      <p><a href="{home_path}">立即返回首页</a></p>
    </main>
    <script>
      let count = 5;
      const el = document.getElementById('countdown');
      const timer = setInterval(() => {{
        count -= 1;
        if (el) el.textContent = String(count);
        if (count <= 0) {{
          clearInterval(timer);
          window.location.replace('{home_path}');
        }}
      }}, 1000);
    </script>
  </body>
</html>"#
        )),
    )
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
