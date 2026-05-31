use crate::common::{is_forbidden_hostname, AppState};
use axum::{
    body::{to_bytes, Body},
    extract::{OriginalUri, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{Html as HtmlResponse, IntoResponse, Response},
};
use bytes::Bytes;
use encoding_rs::{Encoding, UTF_8};
use flate2::read::{GzDecoder, ZlibDecoder};
use std::{
    collections::HashMap,
    io::Read,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use futures_util::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use url::Url;

const WEB_SESSION_COOKIE: &str = "__YUNDO_WEB_SID";
const MAX_REWRITE_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone, Copy)]
enum RewriteKind {
    Html,
    Css,
    JavaScript,
    OtherText,
}

pub async fn web_proxy_handler(
    State(state): State<Arc<AppState>>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(target_url) = target_from_uri(uri.path(), uri.query()) else {
        return web_proxy_landing().into_response();
    };

    let target_url = match normalize_target_url(&target_url) {
        Ok(url) => url,
        Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
    };

    if let Err(message) = validate_target_url(&target_url) {
        return (StatusCode::FORBIDDEN, message).into_response();
    }

    let proxy_prefix = proxy_prefix_from_path(uri.path());
    let session_id = session_id_from_headers(&headers).unwrap_or_else(new_session_id);
    if is_douyin_host(&target_url) && is_document_request(&headers) {
        tracing::info!(
            target_url = %target_url,
            "douyin document navigation detected; redirecting browser to original origin"
        );
        return direct_browser_verification_response(&target_url, &session_id);
    }

    let cookie_scope = cookie_scope(&session_id, &target_url);
    let mut upstream = state
        .web_client
        .request(reqwest_method(&method), target_url.clone())
        .headers(upstream_request_headers(&headers));

    if let Some(cookie_header) = upstream_cookie_header(&state, &cookie_scope, &headers).await {
        upstream = upstream.header(reqwest::header::COOKIE, cookie_header);
    }

    if method_allows_body(&method) {
        match to_bytes(body, MAX_REWRITE_BYTES).await {
            Ok(bytes) if !bytes.is_empty() => upstream = upstream.body(bytes),
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(error = %err, "failed to read proxied request body");
                return (StatusCode::BAD_REQUEST, "failed to read request body").into_response();
            }
        }
    }

    let upstream_response = match upstream.send().await {
        Ok(response) => response,
        Err(err) => {
            tracing::warn!(target_url = %target_url, error = %err, "web proxy upstream request failed");
            ::metrics::counter!("yundo_proxy_upstream_errors_total", "proxy" => "web").increment(1);
            return (StatusCode::BAD_GATEWAY, "failed to reach target server").into_response();
        }
    };

    persist_upstream_cookies(&state, &cookie_scope, upstream_response.headers()).await;

    if is_cloudflare_challenge(upstream_response.headers()) && is_document_request(&headers) {
        tracing::info!(
            target_url = %target_url,
            "cloudflare challenge detected; redirecting browser to original origin"
        );
        return direct_browser_verification_response(&target_url, &session_id);
    }

    if upstream_response.status().is_redirection() {
        return redirect_response(
            upstream_response.status(),
            upstream_response.headers(),
            &target_url,
            &proxy_prefix,
            &session_id,
        );
    }

    let status = upstream_response.status();
    let content_type = upstream_response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let content_encoding = upstream_response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let expose_target_path = should_expose_target_path(upstream_response.headers(), &target_url);
    let rewrite_kind =
        rewrite_kind(&content_type).map(|kind| effective_rewrite_kind(kind, &headers));
    let mut response_headers =
        response_headers(upstream_response.headers(), rewrite_kind.is_some());

    if let Some(kind) = rewrite_kind {
        set_utf8_content_type(&mut response_headers, &content_type, &kind);

        let bytes = match upstream_response.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(target_url = %target_url, error = %err, "failed to read web proxy response");
                ::metrics::counter!("yundo_proxy_upstream_errors_total", "proxy" => "web_response")
                    .increment(1);
                return (StatusCode::BAD_GATEWAY, "failed to read target response").into_response();
            }
        };

        if bytes.len() > MAX_REWRITE_BYTES {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                "response is too large for web page rewriting",
            )
                .into_response();
        }

        let bytes = decompress_text_bytes(bytes, &content_encoding);
        let text = decode_text_response(&bytes, &content_type, kind);
        if matches!(kind, RewriteKind::Html)
            && is_document_request(&headers)
            && is_douyin_verification_page(&target_url, &text)
        {
            tracing::info!(
                target_url = %target_url,
                "douyin verification page detected; redirecting browser to original origin"
            );
            return direct_browser_verification_response(&target_url, &session_id);
        }

        let rewritten =
            rewrite_text_response(&text, kind, &target_url, &proxy_prefix, expose_target_path);
        return response_with_session_cookie(status, response_headers, rewritten, &session_id);
    }

    let mut stream = upstream_response.bytes_stream();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(16);
    tokio::spawn(async move {
        while let Some(chunk) = stream.next().await {
            let mapped = chunk.map_err(|err| std::io::Error::other(err.to_string()));
            if tx.send(mapped).await.is_err() {
                break;
            }
        }
    });

    response_with_session_cookie(
        status,
        response_headers,
        Body::from_stream(ReceiverStream::new(rx)),
        &session_id,
    )
}

fn target_from_uri(path: &str, query: Option<&str>) -> Option<String> {
    if let Some(query) = query {
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if key == "url" && !value.trim().is_empty() {
                return Some(value.into_owned());
            }
        }
    }

    let marker = "/browse/";
    let index = path.find(marker)?;
    let mut target = percent_decode(&path[index + marker.len()..]).ok()?;
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        if target.contains('?') {
            target.push('&');
        } else {
            target.push('?');
        }
        target.push_str(query);
    }
    Some(target)
}

fn normalize_target_url(input: &str) -> Result<Url, &'static str> {
    let normalized = if input.starts_with("http://") || input.starts_with("https://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };

    Url::parse(&normalized).map_err(|_| "invalid URL format")
}

fn validate_target_url(url: &Url) -> Result<(), &'static str> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only HTTP and HTTPS URLs are supported");
    }

    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host.is_empty() || is_forbidden_hostname(&host) {
        return Err("access to local or private networks is forbidden");
    }

    Ok(())
}

fn proxy_prefix_from_path(path: &str) -> String {
    let marker = "/browse";
    let Some(index) = path.find(marker) else {
        return "/browse".to_string();
    };
    path[..index + marker.len()].to_string()
}

fn reqwest_method(method: &Method) -> reqwest::Method {
    reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET)
}

fn method_allows_body(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD)
}

fn upstream_request_headers(headers: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut result = reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "host"
                | "connection"
                | "content-length"
                | "cookie"
                | "accept-encoding"
                | "sec-fetch-dest"
                | "sec-fetch-mode"
                | "sec-fetch-site"
                | "sec-fetch-user"
        ) {
            continue;
        }
        if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
            result.insert(header_name, value.clone());
        }
    }
    if !result.contains_key(reqwest::header::USER_AGENT) {
        result.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_static("yundo-web-proxy/1.0"),
        );
    }
    result
}

fn response_headers(headers: &reqwest::header::HeaderMap, rewritten: bool) -> HeaderMap {
    let mut result = HeaderMap::new();
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "connection"
                | "content-encoding"
                | "content-security-policy"
                | "content-security-policy-report-only"
                | "cross-origin-embedder-policy"
                | "cross-origin-opener-policy"
                | "cross-origin-resource-policy"
                | "permissions-policy"
                | "set-cookie"
                | "transfer-encoding"
        ) {
            continue;
        }
        if rewritten && lower == "content-length" {
            continue;
        }
        if let Ok(header_name) = HeaderName::from_bytes(name.as_str().as_bytes()) {
            result.insert(header_name, value.clone());
        }
    }
    result
}

fn should_expose_target_path(headers: &reqwest::header::HeaderMap, target_url: &Url) -> bool {
    target_url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("douyu.com") || host.ends_with(".douyu.com"))
        || is_cloudflare_challenge(headers)
}

fn is_douyin_verification_page(target_url: &Url, text: &str) -> bool {
    is_douyin_host(target_url)
        && text.contains("verifycenter/captcha")
        && text.contains("rmc-captcha")
}

fn is_douyin_host(target_url: &Url) -> bool {
    target_url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("douyin.com") || host.ends_with(".douyin.com")
    })
}

fn is_cloudflare_challenge(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get("cf-mitigated")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("challenge"))
}

fn is_document_request(headers: &HeaderMap) -> bool {
    let sec_fetch_dest = headers
        .get("sec-fetch-dest")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if sec_fetch_dest.eq_ignore_ascii_case("document")
        || sec_fetch_dest.eq_ignore_ascii_case("iframe")
    {
        return true;
    }

    let upgrade_insecure_requests = headers
        .get("upgrade-insecure-requests")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if upgrade_insecure_requests == "1" {
        return true;
    }

    if !headers.contains_key(header::REFERER) {
        return true;
    }

    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/html"))
}

fn set_utf8_content_type(headers: &mut HeaderMap, original_content_type: &str, kind: &RewriteKind) {
    let media_type = original_content_type
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(match kind {
            RewriteKind::Html => "text/html",
            RewriteKind::Css => "text/css",
            RewriteKind::JavaScript => "application/javascript",
            RewriteKind::OtherText => "text/plain",
        });

    if let Ok(value) = HeaderValue::from_str(&format!("{media_type}; charset=utf-8")) {
        headers.insert(header::CONTENT_TYPE, value);
    }
}

fn decode_text_response(bytes: &[u8], content_type: &str, kind: RewriteKind) -> String {
    let label = charset_from_content_type(content_type).or_else(|| {
        matches!(kind, RewriteKind::Html)
            .then(|| charset_from_html_meta(bytes))
            .flatten()
    });
    let encoding = label
        .as_deref()
        .and_then(|label| Encoding::for_label(label.as_bytes()))
        .unwrap_or(UTF_8);
    let (decoded, _, _) = encoding.decode(bytes);
    decoded.into_owned()
}

fn decompress_text_bytes(bytes: Bytes, content_encoding: &str) -> Bytes {
    let lower = content_encoding.to_ascii_lowercase();
    if lower.contains("gzip") || bytes.starts_with(&[0x1f, 0x8b]) {
        return decompress_with(GzDecoder::new(bytes.as_ref())).unwrap_or(bytes);
    }
    if lower.contains("deflate") {
        return decompress_with(ZlibDecoder::new(bytes.as_ref())).unwrap_or(bytes);
    }
    bytes
}

fn decompress_with<R: Read>(mut reader: R) -> Result<Bytes, std::io::Error> {
    let mut output = Vec::new();
    reader.read_to_end(&mut output)?;
    Ok(Bytes::from(output))
}

fn charset_from_content_type(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| {
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string()
            })
            .filter(|value| !value.is_empty())
    })
}

fn charset_from_html_meta(bytes: &[u8]) -> Option<String> {
    let preview_len = bytes.len().min(4096);
    let preview = String::from_utf8_lossy(&bytes[..preview_len]).to_ascii_lowercase();
    if let Some(index) = preview.find("charset=") {
        let after = &preview[index + "charset=".len()..];
        let charset = after
            .trim_start_matches(['"', '\'', ' '])
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '-' | '_' | '.'))
            .collect::<String>();
        if !charset.is_empty() {
            return Some(charset);
        }
    }
    None
}

fn rewrite_kind(content_type: &str) -> Option<RewriteKind> {
    let content_type = content_type.to_ascii_lowercase();
    if content_type.contains("text/html") {
        Some(RewriteKind::Html)
    } else if content_type.contains("text/css") {
        Some(RewriteKind::Css)
    } else if content_type.contains("javascript") || content_type.contains("ecmascript") {
        Some(RewriteKind::JavaScript)
    } else if content_type.starts_with("text/") || content_type.contains("application/json") {
        Some(RewriteKind::OtherText)
    } else {
        None
    }
}

fn effective_rewrite_kind(kind: RewriteKind, request_headers: &HeaderMap) -> RewriteKind {
    if matches!(kind, RewriteKind::Html) && !is_document_request(request_headers) {
        return RewriteKind::OtherText;
    }
    kind
}

fn rewrite_text_response(
    text: &str,
    kind: RewriteKind,
    base: &Url,
    proxy_prefix: &str,
    expose_target_path: bool,
) -> String {
    match kind {
        RewriteKind::Html => rewrite_html(text, base, proxy_prefix, expose_target_path),
        RewriteKind::Css => rewrite_css(text, base, proxy_prefix),
        RewriteKind::JavaScript | RewriteKind::OtherText => text.to_string(),
    }
}

fn rewrite_html(input: &str, base: &Url, proxy_prefix: &str, expose_target_path: bool) -> String {
    let mut output = input.to_string();
    output = normalize_html_charset_meta(&output);
    output = rewrite_html_tag_attrs(&output, base, proxy_prefix);
    output = inject_runtime(&output, base, proxy_prefix, expose_target_path);
    output
}

fn rewrite_html_tag_attrs(input: &str, base: &Url, proxy_prefix: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find('<') {
        output.push_str(&rest[..start]);
        let after_start = &rest[start..];
        let Some(end) = after_start.find('>') else {
            output.push_str(after_start);
            return output;
        };

        let tag = &after_start[..=end];
        let rewritten_tag = rewrite_single_tag_attrs(tag, base, proxy_prefix);
        output.push_str(&rewritten_tag);
        rest = &after_start[end + 1..];

        if is_opening_tag(tag, "script") {
            if let Some(close_index) = find_case_insensitive(rest, "</script") {
                output.push_str(&rest[..close_index]);
                rest = &rest[close_index..];
            }
        }
    }

    output.push_str(rest);
    output
}

fn rewrite_single_tag_attrs(tag: &str, base: &Url, proxy_prefix: &str) -> String {
    if tag.starts_with("</") || tag.starts_with("<!--") || tag.starts_with("<!") {
        return tag.to_string();
    }

    let mut output = tag.to_string();
    for attr in ["href", "src", "action", "poster", "data"] {
        output = rewrite_attr_urls(&output, attr, base, proxy_prefix, false);
    }
    output = rewrite_attr_urls(&output, "srcset", base, proxy_prefix, true);
    output = rewrite_attr_urls(&output, "imagesrcset", base, proxy_prefix, true);
    remove_integrity_attrs(&output)
}

fn is_opening_tag(tag: &str, name: &str) -> bool {
    let trimmed = tag.trim_start();
    if trimmed.starts_with("</") || trimmed.ends_with("/>") {
        return false;
    }
    let Some(after_lt) = trimmed.strip_prefix('<') else {
        return false;
    };
    let tag_name = after_lt
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    tag_name.eq_ignore_ascii_case(name)
}

fn normalize_html_charset_meta(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find('<') {
        output.push_str(&rest[..start]);
        let after_start = &rest[start..];
        let Some(end) = after_start.find('>') else {
            output.push_str(after_start);
            return output;
        };
        let tag = &after_start[..=end];
        if tag
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("<meta"))
        {
            output.push_str(&normalize_meta_tag_charset(tag));
        } else {
            output.push_str(tag);
        }
        rest = &after_start[end + 1..];
    }
    output.push_str(rest);

    if !has_html_charset_meta(&output) {
        if let Some(index) = find_case_insensitive(&output, "<head") {
            if let Some(end) = output[index..].find('>') {
                output.insert_str(index + end + 1, r#"<meta charset="utf-8">"#);
            }
        }
    }

    output
}

fn has_html_charset_meta(input: &str) -> bool {
    find_case_insensitive(input, "<meta").is_some()
        && find_case_insensitive(input, "charset").is_some()
}

fn normalize_meta_tag_charset(tag: &str) -> String {
    let mut output = rewrite_meta_attr_value(tag, "charset", |_| "utf-8".to_string());
    output = rewrite_meta_attr_value(&output, "content", |value| {
        if let Some(index) = find_case_insensitive(value, "charset=") {
            let value_start = index + "charset=".len();
            let value_end = value[value_start..]
                .find(|ch: char| matches!(ch, ';' | ' ' | '\t' | '\n' | '\r'))
                .map(|end| value_start + end)
                .unwrap_or(value.len());
            let mut replaced = value.to_string();
            replaced.replace_range(value_start..value_end, "utf-8");
            replaced
        } else {
            value.to_string()
        }
    });
    output
}

fn rewrite_meta_attr_value<F>(tag: &str, attr: &str, replace: F) -> String
where
    F: Fn(&str) -> String,
{
    let pattern = format!("{attr}=");
    let Some(index) = find_case_insensitive(tag, &pattern) else {
        return tag.to_string();
    };

    let value_start = index + pattern.len();
    let Some(quote) = tag[value_start..]
        .chars()
        .next()
        .filter(|ch| *ch == '"' || *ch == '\'')
    else {
        return tag.to_string();
    };
    let inner_start = value_start + quote.len_utf8();
    let Some(relative_end) = tag[inner_start..].find(quote) else {
        return tag.to_string();
    };
    let inner_end = inner_start + relative_end;
    let mut output = String::with_capacity(tag.len());
    output.push_str(&tag[..inner_start]);
    output.push_str(&replace(&tag[inner_start..inner_end]));
    output.push_str(&tag[inner_end..]);
    output
}

fn rewrite_attr_urls(
    input: &str,
    attr: &str,
    base: &Url,
    proxy_prefix: &str,
    srcset: bool,
) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    let pattern = format!("{attr}=");

    while let Some(index) = find_case_insensitive(rest, &pattern) {
        output.push_str(&rest[..index]);
        output.push_str(&rest[index..index + pattern.len()]);
        rest = &rest[index + pattern.len()..];

        let Some(quote) = rest.chars().next().filter(|ch| *ch == '"' || *ch == '\'') else {
            let end = rest
                .find(|ch: char| ch.is_whitespace() || ch == '>')
                .unwrap_or(rest.len());
            let value = &rest[..end];
            let rewritten = if srcset {
                rewrite_srcset(value, base, proxy_prefix)
            } else {
                rewrite_url_value(value, base, proxy_prefix)
            };
            output.push_str(&rewritten);
            rest = &rest[end..];
            continue;
        };
        output.push(quote);
        rest = &rest[quote.len_utf8()..];

        let Some(end) = rest.find(quote) else {
            output.push_str(rest);
            return output;
        };
        let value = &rest[..end];
        let rewritten = if srcset {
            rewrite_srcset(value, base, proxy_prefix)
        } else {
            rewrite_url_value(value, base, proxy_prefix)
        };
        output.push_str(&rewritten);
        output.push(quote);
        rest = &rest[end + quote.len_utf8()..];
    }

    output.push_str(rest);
    output
}

fn remove_integrity_attrs(input: &str) -> String {
    let mut output = input.to_string();
    for quote in ['"', '\''] {
        loop {
            let Some(index) = find_case_insensitive(&output, " integrity=") else {
                break;
            };
            let value_start = index + " integrity=".len();
            if output[value_start..].starts_with(quote) {
                if let Some(end) = output[value_start + 1..].find(quote) {
                    output.replace_range(index..value_start + 2 + end, "");
                    continue;
                }
            }
            break;
        }
    }
    output
}

fn rewrite_srcset(value: &str, base: &Url, proxy_prefix: &str) -> String {
    value
        .split(',')
        .map(|candidate| {
            let trimmed = candidate.trim();
            let mut parts = trimmed.splitn(2, char::is_whitespace);
            let url = parts.next().unwrap_or("");
            let descriptor = parts.next().unwrap_or("").trim();
            if descriptor.is_empty() {
                rewrite_url_value(url, base, proxy_prefix)
            } else {
                format!(
                    "{} {}",
                    rewrite_url_value(url, base, proxy_prefix),
                    descriptor
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn rewrite_css(input: &str, base: &Url, proxy_prefix: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(index) = find_case_insensitive(rest, "url(") {
        output.push_str(&rest[..index + 4]);
        rest = &rest[index + 4..];
        let Some(end) = find_css_url_end(rest) else {
            output.push_str(rest);
            return output;
        };
        let raw = rest[..end].trim();
        let quote = raw
            .chars()
            .next()
            .filter(|ch| matches!(ch, '"' | '\''))
            .filter(|first| raw.ends_with(*first));
        let value = match quote {
            Some(ch) if raw.len() >= 2 => &raw[ch.len_utf8()..raw.len() - ch.len_utf8()],
            _ => raw,
        };
        if let Some(ch) = quote {
            output.push(ch);
        }
        output.push_str(&rewrite_css_url_value(value, base, proxy_prefix));
        if let Some(ch) = quote {
            output.push(ch);
        }
        output.push(')');
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    rewrite_css_imports(&output, base, proxy_prefix)
}

fn find_css_url_end(input: &str) -> Option<usize> {
    let trimmed_start = input.len() - input.trim_start().len();
    let trimmed = &input[trimmed_start..];
    let quote = trimmed.chars().next().filter(|ch| matches!(ch, '"' | '\''));

    if let Some(quote) = quote {
        let value_start = trimmed_start + quote.len_utf8();
        let mut escaped = false;
        for (offset, ch) in input[value_start..].char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                let after_quote = value_start + offset + quote.len_utf8();
                let after_whitespace = after_quote + input[after_quote..].len()
                    - input[after_quote..].trim_start().len();
                return input[after_whitespace..]
                    .starts_with(')')
                    .then_some(after_whitespace);
            }
        }
        return None;
    }

    input.find(')')
}

fn rewrite_css_imports(input: &str, base: &Url, proxy_prefix: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0;

    while index < input.len() {
        let rest = &input[index..];
        let Some(next) = find_case_insensitive(rest, "@import") else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..next]);
        let import_start = index + next;
        let after_import = import_start + "@import".len();
        output.push_str(&input[import_start..after_import]);

        let Some(import_end_offset) = input[after_import..].find(';') else {
            output.push_str(&input[after_import..]);
            break;
        };
        let import_end = after_import + import_end_offset;
        let import_body = &input[after_import..import_end];

        if let Some((quote_index, quote)) = import_body
            .char_indices()
            .find(|(_, ch)| matches!(ch, '"' | '\''))
        {
            let url_start = quote_index + quote.len_utf8();
            if let Some(url_end_offset) = import_body[url_start..].find(quote) {
                let url_end = url_start + url_end_offset;
                output.push_str(&import_body[..=quote_index]);
                output.push_str(&rewrite_url_value(
                    &import_body[url_start..url_end],
                    base,
                    proxy_prefix,
                ));
                output.push_str(&import_body[url_end..]);
            } else {
                output.push_str(import_body);
            }
        } else {
            output.push_str(import_body);
        }
        output.push(';');
        index = import_end + 1;
    }

    output
}

fn rewrite_css_url_value(value: &str, base: &Url, proxy_prefix: &str) -> String {
    let trimmed = value.trim();
    if trimmed
        .get(..18)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:image/svg+xml"))
    {
        if let Some((prefix, payload)) = trimmed.split_once(',') {
            return format!("{prefix},{}", urlencoding::encode(payload));
        }
    }

    rewrite_url_value(value, base, proxy_prefix)
}

fn rewrite_url_value(value: &str, base: &Url, proxy_prefix: &str) -> String {
    let trimmed = value.trim();
    let normalized_proxy_prefix = proxy_prefix.trim_end_matches('/');
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed == normalized_proxy_prefix
        || trimmed.starts_with(&format!("{normalized_proxy_prefix}/"))
        || trimmed.starts_with("data:")
        || trimmed.starts_with("mailto:")
        || trimmed.starts_with("tel:")
        || trimmed.starts_with("javascript:")
        || trimmed.starts_with("about:")
        || trimmed.starts_with("blob:")
    {
        return value.to_string();
    }

    match base.join(trimmed) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => proxied_url(&url, proxy_prefix),
        _ => value.to_string(),
    }
}

fn inject_runtime(
    html: &str,
    base: &Url,
    proxy_prefix: &str,
    expose_target_path: bool,
) -> String {
    let runtime = runtime_script(base, proxy_prefix, expose_target_path);
    if let Some(index) = find_case_insensitive(html, "<head>") {
        let mut output = String::with_capacity(html.len() + runtime.len());
        let insert_at = index + "<head>".len();
        output.push_str(&html[..insert_at]);
        output.push_str(&runtime);
        output.push_str(&html[insert_at..]);
        output
    } else if let Some(index) = find_case_insensitive(html, "<head ") {
        let Some(end) = html[index..].find('>').map(|offset| index + offset + 1) else {
            return format!("{runtime}{html}");
        };
        let mut output = String::with_capacity(html.len() + runtime.len());
        output.push_str(&html[..end]);
        output.push_str(&runtime);
        output.push_str(&html[end..]);
        output
    } else {
        format!("{runtime}{html}")
    }
}

fn runtime_script(base: &Url, proxy_prefix: &str, expose_target_path: bool) -> String {
    format!(
        r#"<script>
(function () {{
  const yundoBase = {base:?};
  const yundoPrefix = {prefix:?};
  const yundoTarget = new URL(yundoBase);
  const yundoExposeTargetPath = {expose};
  function proxied(input) {{
    if (input == null) return input;
    let value = input instanceof URL ? input.href : String(input);
    const prefixPath = yundoPrefix.replace(/\/$/, '');
    if (!value || value.startsWith('#') || value === prefixPath || value.startsWith(prefixPath + '/') || /^(data|mailto|tel|javascript|about|blob):/i.test(value)) return input;
    try {{
      const url = new URL(value, yundoBase);
      if (url.origin === window.location.origin && (url.pathname === prefixPath || url.pathname.startsWith(prefixPath + '/'))) {{
        return value;
      }}
      return prefixPath + '/' + encodeURIComponent(url.href);
    }} catch (_) {{
      return input;
    }}
  }}

  function displayedTargetPath(input) {{
    if (input == null) return input;
    try {{
      const url = new URL(String(input), yundoBase);
      if (url.protocol === 'http:' || url.protocol === 'https:') {{
        return url.pathname + url.search + url.hash;
      }}
    }} catch (_) {{}}
    return input;
  }}

  function proxiedSrcset(value) {{
    if (!value) return value;
    return String(value).split(',').map(function(candidate) {{
      const trimmed = candidate.trim();
      const parts = trimmed.split(/\s+/, 2);
      const rewritten = proxied(parts[0]);
      return parts.length > 1 ? rewritten + ' ' + parts[1] : rewritten;
    }}).join(', ');
  }}

  function rewriteElement(element) {{
    if (!element || element.nodeType !== 1) return element;
    const attrs = ['src', 'href', 'action', 'poster', 'data'];
    for (const attr of attrs) {{
      if (element.hasAttribute && element.hasAttribute(attr)) {{
        element.setAttribute(attr, proxied(element.getAttribute(attr)));
      }}
    }}
    for (const attr of ['srcset', 'imagesrcset']) {{
      if (element.hasAttribute && element.hasAttribute(attr)) {{
        element.setAttribute(attr, proxiedSrcset(element.getAttribute(attr)));
      }}
    }}
    return element;
  }}

  const originalSetAttribute = Element.prototype.setAttribute;
  Element.prototype.setAttribute = function(name, value) {{
    const lower = String(name).toLowerCase();
    if (['src', 'href', 'action', 'poster', 'data'].includes(lower)) {{
      value = proxied(value);
    }} else if (lower === 'srcset' || lower === 'imagesrcset') {{
      value = proxiedSrcset(value);
    }}
    return originalSetAttribute.call(this, name, value);
  }};

  function hookUrlProperty(ctor, prop, srcset) {{
    if (!ctor || !ctor.prototype) return;
    const descriptor = Object.getOwnPropertyDescriptor(ctor.prototype, prop);
    if (!descriptor || !descriptor.set || !descriptor.get) return;
    Object.defineProperty(ctor.prototype, prop, {{
      configurable: true,
      enumerable: descriptor.enumerable,
      get: function() {{ return descriptor.get.call(this); }},
      set: function(value) {{ return descriptor.set.call(this, srcset ? proxiedSrcset(value) : proxied(value)); }}
    }});
  }}

  [
    [HTMLAnchorElement, 'href', false],
    [HTMLAreaElement, 'href', false],
    [HTMLLinkElement, 'href', false],
    [HTMLScriptElement, 'src', false],
    [HTMLImageElement, 'src', false],
    [HTMLIFrameElement, 'src', false],
    [HTMLSourceElement, 'src', false],
    [HTMLVideoElement, 'src', false],
    [HTMLAudioElement, 'src', false],
    [HTMLTrackElement, 'src', false],
    [HTMLInputElement, 'src', false],
    [HTMLFormElement, 'action', false],
    [HTMLObjectElement, 'data', false],
    [HTMLImageElement, 'srcset', true],
    [HTMLSourceElement, 'srcset', true],
    [HTMLLinkElement, 'imageSrcset', true]
  ].forEach(function(item) {{ hookUrlProperty(item[0], item[1], item[2]); }});

  const originalAppendChild = Node.prototype.appendChild;
  Node.prototype.appendChild = function(child) {{
    return originalAppendChild.call(this, rewriteElement(child));
  }};
  const originalInsertBefore = Node.prototype.insertBefore;
  Node.prototype.insertBefore = function(newNode, referenceNode) {{
    return originalInsertBefore.call(this, rewriteElement(newNode), referenceNode);
  }};

  const originalFetch = window.fetch;
  window.fetch = function(input, init) {{
    if (input instanceof Request) {{
      return originalFetch(new Request(proxied(input.url), input), init);
    }}
    return originalFetch(proxied(input), init);
  }};

  if (navigator.serviceWorker && navigator.serviceWorker.register) {{
    const blockedServiceWorkerRegistration = function() {{
      return Promise.reject(new DOMException('Service workers are disabled inside the yundo web proxy.', 'SecurityError'));
    }};
    try {{
      Object.defineProperty(navigator.serviceWorker, 'register', {{
        configurable: true,
        value: blockedServiceWorkerRegistration
      }});
    }} catch (_) {{
      navigator.serviceWorker.register = blockedServiceWorkerRegistration;
    }}
  }}

  const originalOpen = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function(method, url) {{
    const args = Array.prototype.slice.call(arguments);
    args[1] = proxied(url);
    return originalOpen.apply(this, args);
  }};

  const originalWindowOpen = window.open;
  window.open = function(url, target, features) {{
    return originalWindowOpen.call(window, proxied(url), target, features);
  }};

  const originalPushState = history.pushState;
  let activePushState = originalPushState;
  const proxiedPushState = function(state, title, url) {{
    return activePushState.call(this, state, title, url == null ? url : (yundoExposeTargetPath ? displayedTargetPath(url) : proxied(url)));
  }};
  const originalReplaceState = history.replaceState;
  let activeReplaceState = originalReplaceState;
  const proxiedReplaceState = function(state, title, url) {{
    return activeReplaceState.call(this, state, title, url == null ? url : (yundoExposeTargetPath ? displayedTargetPath(url) : proxied(url)));
  }};
  try {{
    Object.defineProperty(history, 'pushState', {{
      configurable: true,
      get: function() {{ return proxiedPushState; }},
      set: function(value) {{
        if (typeof value === 'function' && value !== proxiedPushState) activePushState = value;
      }}
    }});
    Object.defineProperty(history, 'replaceState', {{
      configurable: true,
      get: function() {{ return proxiedReplaceState; }},
      set: function(value) {{
        if (typeof value === 'function' && value !== proxiedReplaceState) activeReplaceState = value;
      }}
    }});
  }} catch (_) {{
    history.pushState = proxiedPushState;
    history.replaceState = proxiedReplaceState;
  }}

  if (yundoExposeTargetPath) {{
    document.cookie = '__YUNDO_WEB_TARGET=' + encodeURIComponent(yundoBase) + '; Path=/; SameSite=Lax';
    originalReplaceState.call(history, history.state, document.title, yundoTarget.pathname + yundoTarget.search + yundoTarget.hash);
  }}

  function restoreProxyLocation() {{
    if (yundoExposeTargetPath) {{
      if (window.location.pathname === yundoPrefix || window.location.pathname.startsWith(yundoPrefix.replace(/\/$/, '') + '/')) {{
        originalReplaceState.call(history, history.state, document.title, yundoTarget.pathname + yundoTarget.search + yundoTarget.hash);
      }}
      return;
    }}
    const prefixPath = yundoPrefix.replace(/\/$/, '');
    if (window.location.pathname === prefixPath || window.location.pathname.startsWith(prefixPath + '/')) return;
    const escapedTarget = window.location.pathname + window.location.search + window.location.hash;
    originalReplaceState.call(history, history.state, document.title, proxied(escapedTarget));
  }}
  window.addEventListener('DOMContentLoaded', restoreProxyLocation);
  window.addEventListener('load', restoreProxyLocation);
  setTimeout(restoreProxyLocation, 0);

  document.addEventListener('click', function(event) {{
    const link = event.target && event.target.closest ? event.target.closest('a[href], area[href]') : null;
    if (!link) return;
    const raw = link.getAttribute('href');
    if (!raw || raw.startsWith('#') || /^(javascript|mailto|tel|data|blob):/i.test(raw)) return;
    const rewritten = proxied(raw);
    event.preventDefault();
    event.stopImmediatePropagation();
    if (link.target === '_blank') {{
      window.open(rewritten, '_blank');
    }} else {{
      window.location.href = rewritten;
    }}
  }}, true);

  document.addEventListener('submit', function(event) {{
    const form = event.target;
    if (form && form.tagName === 'FORM' && form.action) {{
      form.action = proxied(form.action);
    }}
  }}, true);
}})();
</script>"#,
        base = base.as_str(),
        prefix = proxy_prefix,
        expose = expose_target_path
    )
}

fn proxied_url(url: &Url, proxy_prefix: &str) -> String {
    format!(
        "{}/{}",
        proxy_prefix.trim_end_matches('/'),
        percent_encode(url.as_str())
    )
}

fn redirect_response(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    base: &Url,
    proxy_prefix: &str,
    session_id: &str,
) -> Response {
    let mut response_headers = response_headers(headers, true);
    if let Some(location) = headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Ok(target) = base.join(location) {
            if let Ok(value) = HeaderValue::from_str(&proxied_url(&target, proxy_prefix)) {
                response_headers.insert(header::LOCATION, value);
            }
        }
    }
    response_with_session_cookie(status, response_headers, Body::empty(), session_id)
}

fn response_with_session_cookie<B>(
    status: reqwest::StatusCode,
    mut headers: HeaderMap,
    body: B,
    session_id: &str,
) -> Response
where
    B: Into<Body>,
{
    let cookie = format!("{WEB_SESSION_COOKIE}={session_id}; Path=/; SameSite=Lax; HttpOnly");
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.append(header::SET_COOKIE, value);
    }
    (
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
        headers,
        body.into(),
    )
        .into_response()
}

fn direct_browser_verification_response(target_url: &Url, session_id: &str) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(target_url.as_str()) {
        headers.insert(header::LOCATION, value);
    }
    response_with_session_cookie(
        reqwest::StatusCode::FOUND,
        headers,
        Body::from("Browser verification requires direct origin navigation."),
        session_id,
    )
}

fn web_proxy_landing() -> HtmlResponse<&'static str> {
    HtmlResponse(
        r#"<!doctype html>
<html lang="zh-CN">
  <head><meta charset="utf-8"><title>Yundo Web Proxy</title></head>
  <body>
    <form method="get" action="/browse">
      <input name="url" placeholder="https://example.com" style="width: 360px">
      <button type="submit">Open</button>
    </form>
  </body>
</html>"#,
    )
}

fn session_id_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == WEB_SESSION_COOKIE && !value.is_empty()).then(|| value.to_string())
    })
}

fn new_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}")
}

fn cookie_scope(session_id: &str, url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    format!("{session_id}|{}://{}{}", url.scheme(), host, port)
}

async fn upstream_cookie_header(
    state: &AppState,
    scope: &str,
    request_headers: &HeaderMap,
) -> Option<String> {
    let mut merged = HashMap::new();

    let mut cookies = state.web_cookies.lock().await;
    if let Some(scoped) = cookies.get(scope) {
        for (name, value) in scoped {
            merged.insert(name.clone(), value.clone());
        }
    }
    drop(cookies);

    if let Some(browser_cookie) = request_headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
    {
        for part in browser_cookie.split(';') {
            let Some((name, value)) = part.trim().split_once('=') else {
                continue;
            };
            let name = name.trim();
            if is_internal_proxy_cookie(name) || name.is_empty() {
                continue;
            }
            merged
                .entry(name.to_string())
                .or_insert_with(|| value.trim().to_string());
        }
    }

    if merged.is_empty() {
        return None;
    }

    Some(
        merged
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

fn is_internal_proxy_cookie(name: &str) -> bool {
    name == WEB_SESSION_COOKIE || name == "__YUNDO_WEB_TARGET"
}

async fn persist_upstream_cookies(
    state: &AppState,
    scope: &str,
    headers: &reqwest::header::HeaderMap,
) {
    let set_cookies = headers.get_all(reqwest::header::SET_COOKIE);
    let mut parsed = Vec::new();
    for value in set_cookies {
        let Ok(value) = value.to_str() else {
            continue;
        };
        let Some(first) = value.split(';').next() else {
            continue;
        };
        let Some((name, cookie_value)) = first.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if !name.is_empty() {
            parsed.push((name.to_string(), cookie_value.trim().to_string()));
        }
    }

    if parsed.is_empty() {
        return;
    }

    let mut cookies = state.web_cookies.lock().await;

    // Get or create the HashMap for this scope
    if let Some(scoped) = cookies.get_mut(scope) {
        for (name, value) in parsed {
            scoped.insert(name, value);
        }
    } else {
        let mut new_map = HashMap::new();
        for (name, value) in parsed {
            new_map.insert(name, value);
        }
        cookies.put(scope.to_string(), new_map);
    }
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

fn percent_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn percent_decode(input: &str) -> Result<String, ()> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).map_err(|_| ())?;
                decoded.push(u8::from_str_radix(hex, 16).map_err(|_| ())?);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8(decoded).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_html_urls_and_injects_runtime() {
        let base = Url::parse("https://example.com/dir/page.html").unwrap();
        let html = r#"<html><head></head><body><a href="/next">n</a><script src="app.js" integrity="sha256-x"></script></body></html>"#;

        let rewritten = rewrite_html(html, &base, "/browse", false);

        assert!(rewritten.contains("/browse/https%3A%2F%2Fexample.com%2Fnext"));
        assert!(rewritten.contains("/browse/https%3A%2F%2Fexample.com%2Fdir%2Fapp.js"));
        assert!(!rewritten.contains("integrity="));
        assert!(rewritten.contains("XMLHttpRequest.prototype.open"));
        assert!(rewritten.contains("originalOpen.apply(this, args)"));
        assert!(rewritten.contains("Element.prototype.setAttribute"));
        assert!(rewritten.contains("restoreProxyLocation"));
        assert!(rewritten.contains("serviceWorker"));
        assert!(rewritten.contains("__YUNDO_WEB_TARGET"));
        assert!(
            rewritten.find("<head><script>").unwrap()
                < rewritten
                    .find(r#"<a href="/browse/https%3A%2F%2Fexample.com%2Fnext""#)
                    .unwrap()
        );
    }

    #[test]
    fn non_document_html_response_is_not_injected() {
        let base = Url::parse("https://verify.example.test/captcha/get").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::REFERER, HeaderValue::from_static("http://127.0.0.1/browse/"));
        let kind = effective_rewrite_kind(RewriteKind::Html, &headers);

        let rewritten = rewrite_text_response(
            r#"{"code":200,"message":"ok"}"#,
            kind,
            &base,
            "/browse",
            false,
        );

        assert_eq!(rewritten, r#"{"code":200,"message":"ok"}"#);
        assert!(!rewritten.contains("yundoBase"));
    }

    #[test]
    fn navigation_html_response_is_injected() {
        let base = Url::parse("https://example.com/").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("upgrade-insecure-requests", HeaderValue::from_static("1"));
        let kind = effective_rewrite_kind(RewriteKind::Html, &headers);

        let rewritten = rewrite_text_response(
            r#"<html><head></head><body></body></html>"#,
            kind,
            &base,
            "/browse",
            false,
        );

        assert!(rewritten.contains("yundoBase"));
    }

    #[test]
    fn headerless_html_response_defaults_to_document() {
        let base = Url::parse("https://example.com/").unwrap();
        let headers = HeaderMap::new();
        let kind = effective_rewrite_kind(RewriteKind::Html, &headers);

        let rewritten = rewrite_text_response(
            r#"<html><head></head><body><script src="/app.js"></script></body></html>"#,
            kind,
            &base,
            "/browse",
            false,
        );

        assert!(rewritten.contains("yundoBase"));
        assert!(rewritten.contains("/browse/https%3A%2F%2Fexample.com%2Fapp.js"));
    }

    #[test]
    fn detects_douyin_verification_page() {
        let target = Url::parse("https://www.douyin.com/").unwrap();
        let html = r#"<html><script src="https://rmc.bytedance.com/verifycenter/captcha/v2"></script><script src="/obj/rc-verifycenter/rmc-captcha/latest/captcha.js"></script></html>"#;

        assert!(is_douyin_verification_page(&target, html));
    }

    #[test]
    fn ignores_non_douyin_captcha_pages() {
        let target = Url::parse("https://example.com/").unwrap();
        let html = r#"<html><script src="https://rmc.bytedance.com/verifycenter/captcha/v2"></script><script src="/obj/rc-verifycenter/rmc-captcha/latest/captcha.js"></script></html>"#;

        assert!(!is_douyin_verification_page(&target, html));
    }

    #[test]
    fn detects_douyin_hosts() {
        assert!(is_douyin_host(
            &Url::parse("https://douyin.com/").unwrap()
        ));
        assert!(is_douyin_host(
            &Url::parse("https://www.douyin.com/").unwrap()
        ));
        assert!(!is_douyin_host(
            &Url::parse("https://notdouyin.com/").unwrap()
        ));
    }

    #[test]
    fn path_target_keeps_get_form_query() {
        let target = target_from_uri(
            "/browse/https%3A%2F%2Fwww.google.com%2Fsearch",
            Some("q=OpenAI+test&hl=zh-CN"),
        )
        .unwrap();

        assert_eq!(
            target,
            "https://www.google.com/search?q=OpenAI+test&hl=zh-CN"
        );
    }

    #[test]
    fn query_url_takes_precedence_over_path_query() {
        let target = target_from_uri(
            "/browse/https%3A%2F%2Fexample.com%2Fignored",
            Some("url=https%3A%2F%2Fexample.com%2Ftarget%3Fx%3D1&q=ignored"),
        )
        .unwrap();

        assert_eq!(target, "https://example.com/target?x=1");
    }

    #[test]
    fn rewrites_unquoted_html_attrs() {
        let base = Url::parse("https://example.com/dir/page.html").unwrap();
        let html = r#"<script src=/xjs/_/js/app.js></script>"#;

        let rewritten = rewrite_html(html, &base, "/browse", false);

        assert!(rewritten.contains("/browse/https%3A%2F%2Fexample.com%2Fxjs%2F_%2Fjs%2Fapp.js"));
    }

    #[test]
    fn does_not_rewrite_script_text() {
        let base = Url::parse("https://example.com/dir/page.html").unwrap();
        let html = r#"<script>const text = 'src=/not-an-attr'; const markup = '<a href="/must-stay-raw">x</a>'; const url = 'https://example.com/raw'; const re = /https?:\/\//;</script>"#;

        let rewritten = rewrite_html(html, &base, "/browse", false);

        assert!(rewritten.contains("src=/not-an-attr"));
        assert!(rewritten.contains(r#"<a href="/must-stay-raw">x</a>"#));
        assert!(rewritten.contains("https://example.com/raw"));
        assert!(rewritten.contains(r#"/https?:\/\//"#));
    }

    #[test]
    fn leaves_blob_urls_unproxied() {
        let base = Url::parse("https://example.com/").unwrap();

        assert_eq!(
            rewrite_url_value("blob:http://127.0.0.1:8080/id", &base, "/browse"),
            "blob:http://127.0.0.1:8080/id"
        );
    }

    #[test]
    fn rewrites_srcset_candidates() {
        let base = Url::parse("https://example.com/images/").unwrap();

        let rewritten = rewrite_srcset("small.jpg 1x, /large.jpg 2x", &base, "/browse");

        assert!(rewritten.contains("/browse/https%3A%2F%2Fexample.com%2Fimages%2Fsmall.jpg 1x"));
        assert!(rewritten.contains("/browse/https%3A%2F%2Fexample.com%2Flarge.jpg 2x"));
    }

    #[test]
    fn keeps_non_http_url_values_unchanged() {
        let base = Url::parse("https://example.com/").unwrap();

        assert_eq!(
            rewrite_url_value("data:image/png;base64,abc", &base, "/browse"),
            "data:image/png;base64,abc"
        );
        assert_eq!(
            rewrite_url_value("javascript:void(0)", &base, "/browse"),
            "javascript:void(0)"
        );
    }

    #[test]
    fn keeps_existing_proxy_url_values_unchanged() {
        let base = Url::parse("https://www.youtube.com/").unwrap();

        assert_eq!(
            rewrite_url_value(
                "/browse/https%3A%2F%2Fwww.youtube.com%2Fs%2Fplayer%2Fbase.js",
                &base,
                "/browse",
            ),
            "/browse/https%3A%2F%2Fwww.youtube.com%2Fs%2Fplayer%2Fbase.js"
        );
    }

    #[test]
    fn css_rewrite_does_not_touch_urls_inside_data_uri_payloads() {
        let base = Url::parse("https://www.douyu.com/").unwrap();
        let css = r#".x{filter:url('data:image/svg+xml;charset=utf-8,<svg xmlns="http://www.w3.org/2000/svg"></svg>#filter');background:url(/a.png)}"#;

        let rewritten = rewrite_css(css, &base, "/browse");

        assert!(rewritten.contains(r#"url('data:image/svg+xml;charset=utf-8,%3Csvg"#));
        assert!(rewritten.contains("%23filter')"));
        assert!(!rewritten.contains("/browse/http%3A%2F%2Fwww.w3.org"));
        assert!(
            rewritten.contains(r#"background:url(/browse/https%3A%2F%2Fwww.douyu.com%2Fa.png)"#)
        );
    }

    #[test]
    fn css_rewrite_rewrites_quoted_import_urls() {
        let base = Url::parse("https://www.douyu.com/styles/main.css").unwrap();

        let rewritten = rewrite_css(r#"@import "/theme.css";.x{color:red}"#, &base, "/browse");

        assert!(rewritten.contains(r#"@import "/browse/https%3A%2F%2Fwww.douyu.com%2Ftheme.css";"#));
        assert!(rewritten.contains(".x{color:red}"));
    }

    #[test]
    fn rewritten_text_responses_are_marked_utf8() {
        let mut headers = HeaderMap::new();

        set_utf8_content_type(
            &mut headers,
            "text/html; charset=GB2312",
            &RewriteKind::Html,
        );

        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
    }

    #[test]
    fn decodes_iso_8859_1_before_rewriting() {
        let decoded = decode_text_response(
            b"<html><head><meta charset=\"ISO-8859-1\"></head><body>\xA3</body></html>",
            "text/html; charset=ISO-8859-1",
            RewriteKind::Html,
        );

        assert!(decoded.contains("£"));
        assert!(!decoded.contains('\u{FFFD}'));
    }

    #[test]
    fn normalizes_html_meta_charset_to_utf8() {
        let html = r#"<html><head><meta charset="ISO-8859-1"><meta http-equiv="content-type" content="text/html; charset=ISO-8859-1"></head></html>"#;

        let normalized = normalize_html_charset_meta(html);

        assert!(normalized.contains(r#"charset="utf-8""#));
        assert!(normalized.contains("charset=utf-8"));
        assert!(!normalized.contains("ISO-8859-1"));
    }

    #[test]
    fn normalizes_content_type_meta_without_breaking_quotes() {
        let html = r#"<html><head><meta content="text/html; charset=ISO-8859-1" http-equiv="Content-Type"></head></html>"#;

        let normalized = normalize_html_charset_meta(html);

        assert!(
            normalized.contains(r#"content="text/html; charset=utf-8" http-equiv="Content-Type""#)
        );
        assert!(!normalized.contains(r#"charset=utf-8 http-equiv="Content-Type"#));
    }
}
