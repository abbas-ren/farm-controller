//! Map [`DeviceRepositoryError`] to Axum HTTP responses for device handlers.
//!
//! Used by every device route handler after a repository / store call fails.
//! Mirrors the status-code conventions of the Node Express error middleware in
//! `farm_controller_old/device-service`.

use super::error::DeviceRepositoryError;
use crate::error::ErrorResponse;
use axum::{Json, http::StatusCode, response::IntoResponse};

/// PURPOSE: Translate a repository error into the matching HTTP status and body.
/// USE: Handler `Err` arms call this instead of matching variants themselves.
pub(crate) fn repository_error(
    error: DeviceRepositoryError,
    message: &str,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::http_support", "repository_error");
    match error {
        DeviceRepositoryError::Validation(message) => {
            error_response(StatusCode::BAD_REQUEST, "validation", &message)
        }
        DeviceRepositoryError::NotFound(message) => {
            error_response(StatusCode::NOT_FOUND, "not_found", &message)
        }
        DeviceRepositoryError::Conflict(message) => {
            error_response(StatusCode::CONFLICT, "conflict", &message)
        }
        DeviceRepositoryError::Internal(error) => {
            tracing::error!(%error, "device repository operation failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
        }
        DeviceRepositoryError::Database(error) => {
            tracing::error!(%error, "device repository operation failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
        }
    }
}

/// PURPOSE: Build a standard `{code, message}` JSON error response.
/// USE: Shared by `repository_error` and handlers that fail before hitting the repository.
pub(crate) fn error_response(
    status: StatusCode,
    code: &str,
    message: &str,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::http_support", "error_response");
    crate::observability::api_fail("error_response", status.as_u16(), message);
    (
        status,
        Json(ErrorResponse {
            code: code.to_owned(),
            message: message.to_owned(),
            request_id: None,
        }),
    )
        .into_response()
}
