use super::error::DeviceRepositoryError;
use crate::error::ErrorResponse;
use axum::{Json, http::StatusCode, response::IntoResponse};

pub(crate) fn repository_error(
    error: DeviceRepositoryError,
    message: &str,
) -> axum::response::Response {
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

pub(crate) fn error_response(
    status: StatusCode,
    code: &str,
    message: &str,
) -> axum::response::Response {
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
