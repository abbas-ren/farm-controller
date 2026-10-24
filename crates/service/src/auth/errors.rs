//! Shared identity and authorization errors and their legacy HTTP envelope.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug)]
pub struct IdentityError {
    pub status: StatusCode,
    pub message: String,
}

pub struct AuthorizationError {
    pub(super) status: StatusCode,
    pub(super) message: String,
}

impl IntoResponse for AuthorizationError {
    /// Into response.
    fn into_response(self) -> Response {
        let _api_trace = crate::observability::ApiTrace::enter("auth::errors", "into_response");
        legacy_error(self.status, &self.message)
    }
}

impl IdentityError {
    /// Upstream.
    pub(super) fn upstream(status: StatusCode, message: impl Into<String>) -> Self {
        let _api_trace = crate::observability::ApiTrace::enter("auth::errors", "upstream");
        Self {
            status,
            message: message.into(),
        }
    }
}

/// Legacy error.
pub(super) fn legacy_error(status: StatusCode, message: &str) -> Response {
    let _api_trace = crate::observability::ApiTrace::enter("auth::errors", "legacy_error");
    (
        status,
        Json(serde_json::json!({ "success": false, "message": message })),
    )
        .into_response()
}
