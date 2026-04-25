use crate::{common::AppState, history::load_ranked_history_items};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};
use std::sync::Arc;

pub async fn robots_txt_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let base_url = derive_base_url(&headers, &state.base_path);
    let body = format!(
        "User-agent: *\nAllow: /\nAllow: /proxydash\nAllow: /downloads\nAllow: /downloads/\nDisallow: /api/\nDisallow: /healthz\nSitemap: {base_url}/sitemap.xml\n"
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
}

pub async fn sitemap_xml_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let base_url = derive_base_url(&headers, &state.base_path);
    let detail_entries = load_ranked_history_items(&state.db, None)
        .await
        .into_iter()
        .map(|item| {
            format!(
                "  <url>\n    <loc>{base_url}/downloads/{slug}</loc>\n    <changefreq>daily</changefreq>\n    <priority>0.7</priority>\n  </url>",
                slug = xml_escape(&item.slug)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>{base_url}/</loc>
    <changefreq>daily</changefreq>
    <priority>1.0</priority>
  </url>
  <url>
    <loc>{base_url}/proxydash</loc>
    <changefreq>hourly</changefreq>
    <priority>0.8</priority>
  </url>
  <url>
    <loc>{base_url}/downloads</loc>
    <changefreq>hourly</changefreq>
    <priority>0.9</priority>
  </url>
{detail_entries}
</urlset>"#
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        body,
    )
}

pub fn derive_base_url(headers: &HeaderMap, configured_base_path: &str) -> String {
    let forwarded_proto = header_value(headers, "x-forwarded-proto");
    let forwarded_host = header_value(headers, "x-forwarded-host");
    let host = header_value(headers, "host").unwrap_or_else(|| "localhost:8080".to_string());

    let proto = forwarded_proto.as_deref().unwrap_or("http");
    let host = forwarded_host.unwrap_or(host);
    let base_path = derive_external_base_path(headers, configured_base_path);

    if base_path == "/" {
        format!("{proto}://{host}")
    } else {
        format!("{proto}://{host}{base_path}")
    }
}

pub fn derive_external_base_path(headers: &HeaderMap, configured_base_path: &str) -> String {
    header_value(headers, "x-forwarded-prefix")
        .and_then(|value| normalize_base_path(&value))
        .unwrap_or_else(|| normalize_base_path(configured_base_path).unwrap_or_else(|| "/".to_string()))
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
    headers.get(name).and_then(|value| value.to_str().ok()).map(|value| {
        value
            .split(',')
            .next()
            .unwrap_or(value)
            .trim()
            .to_string()
    })
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

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
