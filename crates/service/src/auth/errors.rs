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
    fn into_response(self) -> Response {
        legacy_error(self.status, &self.message)
    }
}

impl IdentityError {
    pub(super) fn upstream(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

pub(super) fn legacy_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({ "success": false, "message": message })),
    )
        .into_response()
}
