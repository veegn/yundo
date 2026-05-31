/// Unified error types for the application.
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Invalid URL format")]
    InvalidUrl,

    #[error("Only HTTP and HTTPS URLs are supported")]
    UnsupportedScheme,

    #[error("Access to local or private networks is forbidden")]
    ForbiddenHost,

    #[error("File not found or expired")]
    FileNotFound,

    #[error("File size exceeds maximum allowed size")]
    FileTooLarge,

    #[error("Storage space insufficient")]
    StorageFull,

    #[error("Invalid upload ID")]
    InvalidUploadId,

    #[error("Missing chunk: {0}")]
    MissingChunk(usize),

    #[error("No valid files detected")]
    NoValidFiles,

    #[error("Failed to reach target server")]
    UpstreamConnectionFailed,

    #[error("Upstream server returned error")]
    UpstreamError,

    #[error("Response is too large for web page rewriting")]
    ResponseTooLarge,

    #[error("Failed to read request body")]
    RequestBodyReadFailed,

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Disk I/O error: {0}")]
    DiskError(String),

    #[error("Authentication required")]
    Unauthorized,

    #[error("Invalid API key")]
    InvalidApiKey,

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Internal server error")]
    InternalError,
}

impl AppError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            AppError::InvalidUrl
            | AppError::UnsupportedScheme
            | AppError::InvalidUploadId
            | AppError::NoValidFiles
            | AppError::RequestBodyReadFailed
            | AppError::MissingChunk(_) => StatusCode::BAD_REQUEST,

            AppError::ForbiddenHost => StatusCode::FORBIDDEN,

            AppError::FileNotFound => StatusCode::NOT_FOUND,

            AppError::FileTooLarge | AppError::StorageFull | AppError::ResponseTooLarge => {
                StatusCode::PAYLOAD_TOO_LARGE
            }

            AppError::UpstreamConnectionFailed | AppError::UpstreamError => StatusCode::BAD_GATEWAY,

            AppError::Unauthorized | AppError::InvalidApiKey => StatusCode::UNAUTHORIZED,

            AppError::RateLimitExceeded => StatusCode::TOO_MANY_REQUESTS,

            AppError::DatabaseError(_) | AppError::DiskError(_) | AppError::InternalError => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            AppError::DatabaseError(_) | AppError::DiskError(_) => {
                // Don't expose internal error details to users
                "Internal server error".to_string()
            }
            _ => self.to_string(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let message = self.user_message();

        // Log internal errors
        match &self {
            AppError::DatabaseError(err) => {
                tracing::error!("Database error: {}", err);
            }
            AppError::DiskError(err) => {
                tracing::error!("Disk error: {}", err);
            }
            AppError::InternalError => {
                tracing::error!("Internal error occurred");
            }
            _ => {}
        }

        let body = Json(json!({
            "error": message,
            "status": status.as_u16(),
        }));

        (status, body).into_response()
    }
}

// Convenience conversions
impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::DatabaseError(err.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::DiskError(err.to_string())
    }
}
