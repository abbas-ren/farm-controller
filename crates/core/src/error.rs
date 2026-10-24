//! Shared application error type and HTTP error envelope.
//!
//! Higher crates map domain failures into [`AppError`] so HTTP handlers can
//! return a consistent JSON body without leaking internal details for
//! infrastructure faults. Ports the spirit of `handleControllerError` /
//! `RouteError` helpers from the old Node services into a typed Rust error.

use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use thiserror::Error;
use utoipa::ToSchema;

/// Process-level and bootstrap failures shared across farmcontroller crates.
#[derive(Debug, Error)]
pub enum AppError {
    /// Invalid TOML/env/CLI configuration.
    #[error("invalid configuration: {0}")]
    Configuration(String),
    /// Prometheus registry setup failed.
    #[error("metrics initialization failed: {0}")]
    Metrics(#[from] prometheus::Error),
    /// Tracing / logging subscriber setup failed.
    #[error("telemetry initialization failed: {0}")]
    Telemetry(String),
    /// Keycloak / identity client bootstrap failed.
    #[error("external identity provider initialization failed: {0}")]
    IdentityProvider(String),
    /// sqlx pool or query failure during bootstrap or health checks.
    #[error("database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    /// Embedded migration runner failure.
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

/// JSON body returned for failed HTTP requests.
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
    /// PURPOSE: Map bootstrap/config errors to HTTP status + safe JSON.
    /// USE: Axum invokes this when a handler returns `Err(AppError)`.
    /// Internal variants hide root-cause strings from clients.
    fn into_response(self) -> axum::response::Response {
        let (status, code, message) = match self {
            Self::Configuration(message) => (StatusCode::BAD_REQUEST, "configuration", message),
            // Do not leak sqlx/prometheus/keycloak details to API clients.
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
