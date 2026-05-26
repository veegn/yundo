use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Extract the bearer token from the Authorization header.
pub fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// Axum middleware that enforces internal token authentication.
/// Compares the `Authorization: Bearer <token>` header against the configured internal token.
pub async fn internal_auth_middleware(
    request: Request,
    next: Next,
) -> Response {
    let expected_token = request
        .extensions()
        .get::<InternalToken>()
        .map(|t| t.0.clone());

    let Some(expected) = expected_token else {
        // No internal token configured — reject all internal requests
        return (StatusCode::UNAUTHORIZED, "internal token not configured").into_response();
    };

    let provided = extract_bearer_token(request.headers());
    match provided {
        Some(token) if token == expected => next.run(request).await,
        Some(_) => (StatusCode::FORBIDDEN, "invalid internal token").into_response(),
        None => (StatusCode::UNAUTHORIZED, "missing Authorization header").into_response(),
    }
}

/// Extension type inserted into request extensions to carry the expected internal token.
#[derive(Clone)]
pub struct InternalToken(pub String);
