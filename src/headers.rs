use axum::http::{HeaderMap, HeaderValue};
use url::Url;

/// HTTP headers forwarded from upstream to the client.
pub const ALLOWED_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-disposition",
    "accept-ranges",
    "content-range",
    "etag",
    "last-modified",
];

/// Injects a `Content-Disposition` header if one is not already present.
pub fn ensure_download_filename(headers: &mut HeaderMap, file_name: &str) {
    if headers.contains_key("content-disposition") {
        return;
    }

    if let Ok(value) = HeaderValue::from_str(&build_content_disposition(file_name)) {
        headers.insert("content-disposition", value);
    }
}

/// Resolves the best available file name from response headers or URL segments.
pub fn resolve_file_name(
    original_url: &Url,
    final_url: Option<&Url>,
    headers: &HeaderMap,
) -> String {
    extract_filename_from_headers(headers)
        .or_else(|| final_url.and_then(extract_filename_from_url))
        .or_else(|| extract_filename_from_url(original_url))
        .unwrap_or_else(|| "download.bin".to_string())
}

/// Extracts a file name from a URL path segment or signed URL query parameter.
pub fn extract_filename_from_url(url: &Url) -> Option<String> {
    for (key, value) in url.query_pairs() {
        let key = key.to_ascii_lowercase();
        if (key == "response-content-disposition" || key == "rscd")
            && parse_content_disposition_filename(&value).is_some()
        {
            return parse_content_disposition_filename(&value);
        }
    }

    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
}

/// Builds a standards-compliant `Content-Disposition` header value with both ASCII
/// fallback and RFC 5987 percent-encoded UTF-8 filename.
pub fn build_content_disposition(file_name: &str) -> String {
    let ascii_name = sanitize_ascii_filename(file_name);
    let encoded_name = percent_encode_utf8(file_name);
    format!("attachment; filename=\"{ascii_name}\"; filename*=UTF-8''{encoded_name}")
}

/// Returns `true` for loopback and RFC 1918 private addresses to prevent SSRF.
pub fn is_forbidden_host(host: &str) -> bool {
    host == "localhost"
        || host == "::1"
        || host == "0.0.0.0"
        || host == "127.0.0.1"
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || is_172_private_range(host)
}

fn is_172_private_range(host: &str) -> bool {
    let Some(rest) = host.strip_prefix("172.") else {
        return false;
    };

    let Some(second_octet) = rest.split('.').next() else {
        return false;
    };

    second_octet
        .parse::<u8>()
        .map(|octet| (16..=31).contains(&octet))
        .unwrap_or(false)
}

fn extract_filename_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("content-disposition")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_disposition_filename)
}

fn parse_content_disposition_filename(value: &str) -> Option<String> {
    for part in value.split(';').map(str::trim) {
        if let Some(encoded) = part.strip_prefix("filename*=") {
            let encoded = encoded.strip_prefix("UTF-8''").unwrap_or(encoded);
            if let Ok(decoded) = percent_decode(encoded) {
                let sanitized = sanitize_ascii_filename(&decoded);
                if !sanitized.is_empty() {
                    return Some(decoded);
                }
            }
        }

        if let Some(name) = part.strip_prefix("filename=") {
            let trimmed = name.trim_matches('"').trim_matches('\'').trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

fn percent_decode(value: &str) -> Result<String, ()> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).map_err(|_| ())?;
                let byte = u8::from_str_radix(hex, 16).map_err(|_| ())?;
                decoded.push(byte);
                index += 3;
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8(decoded).map_err(|_| ())
}

pub fn sanitize_ascii_filename(file_name: &str) -> String {
    let sanitized = file_name
        .chars()
        .map(|ch| match ch {
            '"' | '\\' | '/' | ':' | '*' | '?' | '<' | '>' | '|' => '_',
            c if c.is_ascii_graphic() || c == ' ' => c,
            _ => '_',
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string();

    if sanitized.is_empty() {
        "download.bin".to_string()
    } else {
        sanitized
    }
}

fn percent_encode_utf8(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());

    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~' => encoded.push(*byte as char),
            _ => encoded.push_str(&format!("%{:02X}", byte)),
        }
    }

    encoded
}
