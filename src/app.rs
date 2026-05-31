use crate::{
    common::{health_handler, root_handler, AppState},
    history::history_handler,
    metrics::{metrics_handler, track_http_metrics},
    middleware::require_api_key,
    proxy::{proxy_handler, proxy_head_handler},
    web_proxy::web_proxy_handler,
};
use axum::{
    extract::{OriginalUri, State},
    http::{header, HeaderMap, StatusCode},
    middleware,
    response::{Html, IntoResponse, Redirect},
    routing::{any, get},
    Router,
};
use std::{path::PathBuf, sync::Arc};
use tokio::fs;
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};

pub fn build_router(state: Arc<AppState>, frontend_dist: PathBuf) -> Router {
    // Routes that require authentication (if API key is configured)
    let protected_routes = Router::new()
        .route(
            "/api/filebox/upload",
            axum::routing::post(crate::filebox::upload_filebox_handler)
                .layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route(
            "/api/filebox/upload-chunk",
            axum::routing::post(crate::filebox::upload_chunk_handler)
                .layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route(
            "/api/filebox/upload-complete",
            axum::routing::post(crate::filebox::upload_complete_handler)
                .layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route(
            "/api/filebox/upload-abort",
            axum::routing::post(crate::filebox::upload_abort_handler),
        )
        .route(
            "/api/filebox/remote-upload",
            axum::routing::post(crate::filebox::remote_upload_filebox_handler),
        )
        .route(
            "/api/filebox/delete/:id",
            axum::routing::delete(crate::filebox::delete_filebox_handler),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

    let mut inner_router = Router::new()
        .route("/api/proxy", get(proxy_handler).head(proxy_head_handler))
        .route("/browse", any(web_proxy_handler))
        .route("/browse/*target", any(web_proxy_handler))
        .route("/api/recent", get(history_handler))
        .route("/metrics", get(metrics_handler))
        .route(
            "/api/filebox/files",
            get(crate::filebox::list_filebox_handler),
        )
        .route(
            "/api/filebox/download/:id",
            get(crate::filebox::download_filebox_handler),
        )
        .merge(protected_routes)
        .route("/healthz", get(health_handler))
        .layer(CorsLayer::permissive())
        .layer(middleware::from_fn(track_http_metrics))
        .layer(TraceLayer::new_for_http());

    if frontend_dist.join("index.html").exists() {
        tracing::info!("serving frontend assets from {}", frontend_dist.display());
        inner_router = inner_router
            .route("/", get(spa_index_handler))
            .route("/filebox", get(spa_index_handler))
            .route("/webproxy", get(spa_index_handler))
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

    let inner_router = inner_router
        .layer(axum::extract::DefaultBodyLimit::disable())
        .with_state(state.clone());

    let final_router = if state.base_path == "/" {
        inner_router
    } else {
        let redirect_target = state.base_path.clone();
        Router::new()
            .route(
                "/",
                get(move || async move { Redirect::permanent(&redirect_target) }),
            )
            .nest(&state.base_path, inner_router)
    };

    final_router.layer(axum::extract::DefaultBodyLimit::disable())
}

async fn spa_index_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> impl IntoResponse {
    if let Some(redirect) = redirect_escaped_web_proxy_navigation(&headers, &uri, &state.base_path)
    {
        return redirect.into_response();
    }

    let index_path = state.frontend_dist.join("index.html");
    let Ok(template) = fs::read_to_string(&index_path).await else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Frontend index.html could not be read from the configured dist directory.",
        )
            .into_response();
    };

    let request_base_path = derive_external_base_path(&headers, &state.base_path);
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
    if let Some(redirect) = redirect_escaped_web_proxy_navigation(&headers, &uri, &state.base_path)
    {
        return redirect.into_response();
    }

    let base_path = derive_external_base_path(&headers, &state.base_path);
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
        .into_response()
}

fn redirect_escaped_web_proxy_navigation(
    headers: &HeaderMap,
    uri: &axum::http::Uri,
    configured_base_path: &str,
) -> Option<Redirect> {
    let referer = headers.get(header::REFERER)?.to_str().ok()?;
    let referer_url = url::Url::parse(referer).ok()?;
    let target = target_from_browse_referer(&referer_url)
        .or_else(|| target_from_web_proxy_cookie(headers))?;
    let target_url = url::Url::parse(&target).ok()?;
    let escaped_path = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let joined = target_url.join(escaped_path).ok()?;
    let proxy_prefix = prefix_path(configured_base_path, "/browse");
    let redirect_to = format!(
        "{}/{}",
        proxy_prefix.trim_end_matches('/'),
        urlencoding::encode(joined.as_str())
    );
    Some(Redirect::temporary(&redirect_to))
}

fn target_from_web_proxy_cookie(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie.split(';') {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        if name == "__YUNDO_WEB_TARGET" && !value.trim().is_empty() {
            return urlencoding::decode(value)
                .ok()
                .map(|decoded| decoded.into_owned());
        }
    }
    None
}

fn target_from_browse_referer(referer: &url::Url) -> Option<String> {
    let path = referer.path();
    let marker = "/browse/";
    if let Some(index) = path.find(marker) {
        return urlencoding::decode(&path[index + marker.len()..])
            .ok()
            .map(|value| value.into_owned());
    }

    if path.ends_with("/browse") {
        for (key, value) in referer.query_pairs() {
            if key == "url" && !value.trim().is_empty() {
                return Some(value.into_owned());
            }
        }
    }

    None
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn derive_external_base_path(headers: &HeaderMap, configured_base_path: &str) -> String {
    header_value(headers, "x-forwarded-prefix")
        .and_then(|value| normalize_base_path(&value))
        .unwrap_or_else(|| {
            normalize_base_path(configured_base_path).unwrap_or_else(|| "/".to_string())
        })
}

pub fn prefix_path(base_path: &str, path: &str) -> String {
    let normalized_base = normalize_base_path(base_path).unwrap_or_else(|| "/".to_string());
    let normalized_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };

    if normalized_base == "/" {
        normalized_path
    } else if normalized_path == "/" {
        normalized_base
    } else {
        format!("{normalized_base}{normalized_path}")
    }
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(',').next().unwrap_or(value).trim().to_string())
}

fn normalize_base_path(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Some("/".to_string());
    }

    let mut path = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };

    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }

    if path.contains('?') || path.contains('#') {
        return None;
    }

    Some(path)
}
