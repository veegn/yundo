/// Authentication and authorization middleware.
use crate::common::AppState;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

/// Middleware to check API key authentication.
/// If an API key is configured, this middleware validates the request has a matching key.
pub async fn require_api_key(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    // If no API key is configured, allow all requests
    let Some(ref configured_key) = state.api_key else {
        return next.run(request).await;
    };

    // Check for API key in headers
    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    let api_key = if let Some(header) = auth_header {
        // Support both "Bearer <key>" and plain key
        if let Some(key) = header.strip_prefix("Bearer ") {
            Some(key)
        } else {
            Some(header)
        }
    } else {
        // Also check X-API-Key header
        request
            .headers()
            .get("X-API-Key")
            .and_then(|v| v.to_str().ok())
    };

    match api_key {
        Some(key) if key == configured_key => {
            // Valid API key
            next.run(request).await
        }
        Some(_) => {
            // Invalid API key
            (StatusCode::UNAUTHORIZED, "Invalid API key").into_response()
        }
        None => {
            // No API key provided
            (StatusCode::UNAUTHORIZED, "API key required").into_response()
        }
    }
}
