use crate::{
    common::AppState,
    history::load_ranked_history_items,
};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};
use std::sync::Arc;

pub async fn robots_txt_handler(headers: HeaderMap) -> impl IntoResponse {
    let base_url = derive_base_url(&headers);
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
    let base_url = derive_base_url(&headers);
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

pub fn derive_base_url(headers: &HeaderMap) -> String {
    let forwarded_proto = header_value(headers, "x-forwarded-proto");
    let forwarded_host = header_value(headers, "x-forwarded-host");
    let host = header_value(headers, "host").unwrap_or_else(|| "localhost:8080".to_string());

    let proto = forwarded_proto.as_deref().unwrap_or("http");
    let host = forwarded_host.unwrap_or(host);

    format!("{proto}://{host}")
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

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
