use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use thiserror::Error;
use utoipa::ToSchema;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid configuration: {0}")]
    Configuration(String),
    #[error("metrics initialization failed: {0}")]
    Metrics(#[from] prometheus::Error),
    #[error("telemetry initialization failed: {0}")]
    Telemetry(String),
    #[error("external identity provider initialization failed: {0}")]
    IdentityProvider(String),
    #[error("database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Stable machine-readable error category.
    pub code: String,
    /// Safe client-facing description.
    pub message: String,
    /// Request correlation ID when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message) = match self {
            Self::Configuration(message) => (StatusCode::BAD_REQUEST, "configuration", message),
            Self::Metrics(_)
            | Self::Telemetry(_)
            | Self::IdentityProvider(_)
            | Self::Database(_)
            | Self::Migration(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "An internal error occurred".to_owned(),
            ),
        };
        (
            status,
            Json(ErrorResponse {
                code: code.to_owned(),
                message,
                request_id: None,
            }),
        )
            .into_response()
    }
}
