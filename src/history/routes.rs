use super::db::{load_ranked_history_items, to_history_item, RankedHistoryItem, SearchQuery};
use crate::{
    common::AppState,
    seo::{derive_base_url, derive_external_base_path, prefix_path},
};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use std::sync::Arc;
use url::Url;

pub async fn history_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(
        load_ranked_history_items(&state.db, None)
            .await
            .into_iter()
            .map(to_history_item)
            .collect::<Vec<_>>(),
    )
}

pub async fn resources_index_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let items = load_ranked_history_items(&state.db, query.q.as_deref()).await;
    let query_val = query.q.unwrap_or_default();
    let base_url = derive_base_url(&headers, &state.base_path);
    let base_path = derive_external_base_path(&headers, &state.base_path);
    let home_path = prefix_path(&base_path, "/");
    let proxydash_path = prefix_path(&base_path, "/proxydash");
    let downloads_path = prefix_path(&base_path, "/downloads");

    let cards = items
        .iter()
        .map(|item| {
            let detail_path = prefix_path(&base_path, &format!("/downloads/{}", item.slug));
            let download_path = format!(
                "{}?url={}",
                prefix_path(&base_path, "/api/proxy"),
                percent_encode_url(&item.url)
            );

            format!(
                r#"<article class="item-card">
  <h2><a href="{detail_path}">{file_name}</a></h2>
  <p>{description}</p>
  <div class="meta">
    <span>{file_size}</span>
    <span>7 天内 {count_7d} 次下载</span>
    <span>{last_download_at}</span>
  </div>
  <div class="actions">
    <a class="primary" href="{detail_path}">查看详情</a>
    <a class="secondary" href="{download_path}">直接下载</a>
  </div>
</article>"#,
                detail_path = detail_path,
                file_name = html_escape(&item.file_name),
                description = html_escape(&resource_description(item)),
                file_size = human_file_size(item.file_size),
                count_7d = item.count_7d,
                last_download_at = html_escape(&item.last_download_at),
                download_path = download_path,
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let html = format!(
        r#"<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>下载资源列表 | 云渡</title>
    <meta
      name="description"
      content="浏览云渡收录的热门下载资源列表，查看文件详情、热度、大小和最近处理时间。"
    />
    <meta name="robots" content="index,follow" />
    <link rel="canonical" href="{base_url}/downloads" />
    <meta property="og:type" content="website" />
    <meta property="og:title" content="下载资源列表 | 云渡" />
    <meta
      property="og:description"
      content="浏览云渡收录的热门下载资源列表，查看文件详情、热度、大小和最近处理时间。"
    />
    <meta property="og:url" content="{base_url}/downloads" />
    <style>
      body {{
        margin: 0;
        background: linear-gradient(180deg, #f7f9ff 0%, #eef4ff 100%);
        color: #171c22;
        font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }}
      .page {{
        width: min(1120px, calc(100vw - 32px));
        margin: 0 auto;
        padding: 48px 0 72px;
      }}
      .topbar {{
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-bottom: 24px;
      }}
      .brand {{
        color: #171c22;
        font-size: 20px;
        font-weight: 800;
        text-decoration: none;
      }}
      .back {{
        color: #0058bb;
        text-decoration: none;
        font-weight: 700;
        font-size: 14px;
      }}
      .hero {{
        margin-bottom: 24px;
        padding: 32px;
        border-radius: 24px;
        background: rgba(255, 255, 255, 0.92);
        border: 1px solid rgba(194, 198, 214, 0.45);
        box-shadow: 0 24px 80px rgba(23, 28, 34, 0.08);
      }}
      .hero h1 {{
        margin: 0 0 12px;
        font-size: clamp(32px, 5vw, 46px);
      }}
      .hero p {{
        margin: 0 0 24px;
        color: #424753;
        line-height: 1.8;
      }}
      .search-box {{
        position: relative;
        max-width: 540px;
      }}
      .search-box input {{
        width: 100%;
        padding: 16px 20px;
        padding-right: 100px;
        border-radius: 16px;
        border: 1px solid rgba(194, 198, 214, 0.45);
        background: #f8fbff;
        color: #171c22;
        font-size: 16px;
        outline: none;
      }}
      .search-box button {{
        position: absolute;
        right: 8px;
        top: 8px;
        bottom: 8px;
        padding: 0 20px;
        background: #0058bb;
        color: #fff;
        border: none;
        border-radius: 10px;
        font-weight: 700;
        cursor: pointer;
      }}
      .grid {{
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
        gap: 18px;
      }}
      .item-card {{
        padding: 24px;
        border-radius: 22px;
        background: rgba(255, 255, 255, 0.92);
        border: 1px solid rgba(194, 198, 214, 0.45);
        box-shadow: 0 16px 48px rgba(23, 28, 34, 0.07);
      }}
      .item-card h2 {{
        margin: 0 0 12px;
        font-size: 20px;
        line-height: 1.4;
      }}
      .item-card h2 a {{
        color: #171c22;
        text-decoration: none;
        word-break: break-all;
        overflow-wrap: anywhere;
      }}
      .item-card p {{
        margin: 0 0 16px;
        color: #424753;
        line-height: 1.7;
        word-break: break-all;
        overflow-wrap: anywhere;
      }}
      .meta {{
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
        margin-bottom: 18px;
      }}
      .meta span {{
        padding: 6px 10px;
        border-radius: 999px;
        background: #eef4ff;
        color: #0058bb;
        font-size: 12px;
        font-weight: 700;
      }}
      .actions {{
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
      }}
      .actions a {{
        display: inline-flex;
        align-items: center;
        justify-content: center;
        padding: 12px 16px;
        border-radius: 12px;
        text-decoration: none;
        font-weight: 700;
        font-size: 14px;
      }}
      .actions .primary {{
        background: linear-gradient(180deg, #0058bb 0%, #00479a 100%);
        color: #fff;
      }}
      .actions .secondary {{
        background: #eef4ff;
        color: #0058bb;
      }}
    </style>
  </head>
  <body>
    <main class="page">
      <div class="topbar">
        <a class="brand" href="{home_path}">云渡</a>
        <a class="back" href="{proxydash_path}">历史记录</a>
      </div>
      <section class="hero">
        <h1>下载资源列表</h1>
        <p>这里展示站点当前可索引的热门下载资源。每个条目都提供稳定详情页、下载代理入口和基础文件信息。</p>
        <form class="search-box" action="{downloads_path}" method="GET">
          <input type="text" name="q" placeholder="搜索已加速的资源文件名..." value="{query_val}" autocomplete="off">
          <button type="submit">搜索</button>
        </form>
      </section>
      <section class="grid">
        {cards}
      </section>
    </main>
  </body>
</html>"#,
        base_url = base_url,
        home_path = home_path,
        proxydash_path = proxydash_path,
        downloads_path = downloads_path,
        query_val = html_escape(&query_val),
        cards = cards
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

pub async fn resource_detail_handler(
    Path(slug): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let items = load_ranked_history_items(&state.db, None).await;
    let base_path = derive_external_base_path(&headers, &state.base_path);

    let Some(item) = items.iter().find(|item| item.slug == slug) else {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            render_missing_resource_page(&base_path),
        )
            .into_response();
    };

    let base_url = derive_base_url(&headers, &state.base_path);
    let canonical = format!("{base_url}/downloads/{}", item.slug);
    let proxy_url = format!(
        "{}?url={}",
        prefix_path(&base_path, "/api/proxy"),
        percent_encode_url(&item.url)
    );
    let related_links = items
        .iter()
        .filter(|candidate| candidate.slug != item.slug)
        .take(5)
        .map(|candidate| {
            let detail_path = prefix_path(&base_path, &format!("/downloads/{}", candidate.slug));
            format!(
                r#"<li><a href="{detail_path}">{name}</a></li>"#,
                detail_path = detail_path,
                name = html_escape(&candidate.file_name)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let html = format!(
        r#"<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{title} 下载 | 云渡</title>
    <meta name="description" content="{description}" />
    <meta name="robots" content="index,follow" />
    <link rel="canonical" href="{canonical}" />
    <meta property="og:type" content="website" />
    <meta property="og:title" content="{title} 下载 | 云渡" />
    <meta property="og:description" content="{description}" />
    <meta property="og:url" content="{canonical}" />
    <style>
      body {{
        margin: 0;
        background: linear-gradient(180deg, #f7f9ff 0%, #eef4ff 100%);
        color: #171c22;
        font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }}
      .page {{
        width: min(1100px, calc(100vw - 32px));
        margin: 0 auto;
        padding: 48px 0 72px;
      }}
      .topbar {{
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-bottom: 28px;
      }}
      .brand {{
        color: #171c22;
        font-size: 20px;
        font-weight: 800;
        text-decoration: none;
      }}
      .back {{
        color: #0058bb;
        font-size: 14px;
        font-weight: 600;
        text-decoration: none;
      }}
      .hero {{
        display: grid;
        grid-template-columns: minmax(0, 1.8fr) minmax(280px, 1fr);
        gap: 24px;
      }}
      .card {{
        background: rgba(255, 255, 255, 0.92);
        border: 1px solid rgba(194, 198, 214, 0.45);
        border-radius: 24px;
        box-shadow: 0 24px 80px rgba(23, 28, 34, 0.08);
      }}
      .main-card {{
        padding: 32px;
      }}
      .eyebrow {{
        display: inline-flex;
        padding: 6px 12px;
        border-radius: 999px;
        background: #eef4ff;
        color: #0058bb;
        font-size: 12px;
        font-weight: 700;
      }}
      h1 {{
        margin: 18px 0 14px;
        font-size: clamp(32px, 5vw, 48px);
        line-height: 1.08;
        word-break: break-all;
        overflow-wrap: anywhere;
      }}
      .lead {{
        margin: 0 0 26px;
        color: #424753;
        font-size: 16px;
        line-height: 1.8;
        word-break: break-all;
        overflow-wrap: anywhere;
      }}
      .meta-grid {{
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
        gap: 14px;
        margin-bottom: 28px;
      }}
      .meta {{
        padding: 16px 18px;
        border-radius: 18px;
        background: #f8fbff;
        word-break: break-all;
        overflow-wrap: anywhere;
      }}
      .meta strong {{
        display: block;
        margin-bottom: 6px;
        font-size: 12px;
        color: #5f6671;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }}
      .actions {{
        display: flex;
        gap: 12px;
        flex-wrap: wrap;
      }}
      .button {{
        display: inline-flex;
        align-items: center;
        justify-content: center;
        padding: 14px 20px;
        border-radius: 14px;
        background: linear-gradient(180deg, #0058bb 0%, #00479a 100%);
        color: #fff;
        text-decoration: none;
        font-weight: 700;
      }}
      .subtle {{
        display: inline-flex;
        align-items: center;
        justify-content: center;
        padding: 14px 20px;
        border-radius: 14px;
        color: #0058bb;
        background: #eef4ff;
        text-decoration: none;
        font-weight: 700;
      }}
      .side-card {{
        padding: 28px;
      }}
      .side-card h2 {{
        margin: 0 0 14px;
        font-size: 18px;
      }}
      .side-card p {{
        margin: 0 0 14px;
        color: #424753;
        line-height: 1.7;
      }}
      .side-card ul {{
        margin: 0;
        padding-left: 18px;
      }}
      .side-card li + li {{
        margin-top: 10px;
      }}
      .side-card a {{
        color: #0058bb;
        text-decoration: none;
      }}
      .fine-print {{
        margin-top: 24px;
        color: #5f6671;
        font-size: 14px;
        line-height: 1.8;
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
      @media (max-width: 860px) {{
        .hero {{
          grid-template-columns: 1fr;
        }}
      }}
    </style>
  </head>
  <body>
    <main class="page">
      <div class="topbar">
        <a class="brand" href="{home_path}">云渡</a>
        <a class="back" href="{downloads_path}">查看资源列表</a>
      </div>
      <section class="hero">
        <article class="card main-card">
          <span class="eyebrow">下载资源详情</span>
          <h1>{title}</h1>
          <p class="lead">{description}</p>
          <div class="meta-grid">
            <div class="meta">
              <strong>文件名</strong>
              <span>{file_name}</span>
            </div>
            <div class="meta">
              <strong>文件大小</strong>
              <span>{file_size}</span>
            </div>
            <div class="meta">
              <strong>7 天热度</strong>
              <span>{count_7d} 次下载</span>
            </div>
            <div class="meta">
              <strong>最近处理时间</strong>
              <span>{last_download_at}</span>
            </div>
          </div>
          <div class="actions">
            <a class="button" href="{proxy_url}">立即下载</a>
            <a class="subtle" href="{original_url}" rel="nofollow noopener" target="_blank">查看源链接</a>
          </div>
          <p class="fine-print">
            当前资源来自 <code>{source_host}</code>。下载将通过云渡代理转发，并尽量保留原始文件名与断点续传能力。
          </p>
        </article>
        <aside class="card side-card">
          <h2>资源说明</h2>
          <p>该页面由站点历史下载记录自动生成，用于为搜索引擎和访客提供稳定的资源详情入口。</p>
          <p>如果源站支持范围请求，云渡会透传 Range 并支持常见下载器的断点续传。</p>
          <h2>相关资源</h2>
          <ul>{related_links}</ul>
        </aside>
      </section>
    </main>
  </body>
</html>"#,
        title = html_escape(&item.file_name),
        description = html_escape(&resource_description(item)),
        canonical = canonical,
        home_path = prefix_path(&base_path, "/"),
        downloads_path = prefix_path(&base_path, "/downloads"),
        file_name = html_escape(&item.file_name),
        file_size = human_file_size(item.file_size),
        count_7d = item.count_7d,
        last_download_at = html_escape(&item.last_download_at),
        proxy_url = proxy_url,
        original_url = html_escape(&item.url),
        source_host = html_escape(&source_host(&item.url)),
        related_links = related_links
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

fn resource_description(item: &RankedHistoryItem) -> String {
    format!(
        "下载 {}，文件大小 {}，最近 7 天下载 {} 次，最近一次处理时间为 {}。",
        item.file_name,
        human_file_size(item.file_size),
        item.count_7d,
        item.last_download_at
    )
}

fn source_host(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "未知来源".to_string())
}

fn human_file_size(size: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size.max(0) as f64;
    let mut unit = 0;

    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", value as i64, UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

fn render_missing_resource_page(base_path: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>资源不存在 | 云渡</title>
    <meta name="robots" content="noindex,nofollow" />
  </head>
  <body style="font-family: system-ui, sans-serif; padding: 40px; background: #f7f9ff; color: #171c22;">
    <main style="max-width: 640px; margin: 0 auto; background: #fff; border-radius: 20px; padding: 32px;">
      <h1>资源不存在</h1>
      <p>该下载资源当前没有历史记录，可能尚未被创建，或已从站点中移除。</p>
      <p><a href="{home_path}" style="color: #0058bb; font-weight: 700; text-decoration: none;">返回首页</a></p>
    </main>
  </body>
</html>"#,
        home_path = prefix_path(base_path, "/")
    )
}

fn percent_encode_url(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
