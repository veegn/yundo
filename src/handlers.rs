use axum::{
    extract::OriginalUri,
    http::StatusCode,
    response::IntoResponse,
};

pub async fn health_handler() -> &'static str {
    "ok"
}

pub async fn root_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        "Frontend assets are not built yet. Run `npm install` then `npm run build --workspace=frontend`, or call the API routes directly.",
    )
}

pub async fn not_found_handler(OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    let escaped_path = uri
        .path_and_query()
        .map(|value| html_escape(value.as_str()))
        .unwrap_or_else(|| "/".to_string());

    (
        StatusCode::NOT_FOUND,
        [("content-type", "text/html; charset=utf-8")],
        format!(
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
      <p><a href="/">立即返回首页</a></p>
    </main>
    <script>
      let count = 5;
      const el = document.getElementById('countdown');
      const timer = setInterval(() => {{
        count -= 1;
        if (el) el.textContent = String(count);
        if (count <= 0) {{
          clearInterval(timer);
          window.location.replace('/');
        }}
      }}, 1000);
    </script>
  </body>
</html>"#
        ),
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
