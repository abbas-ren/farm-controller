use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::{auth, error::ErrorResponse, state::AppState};

use super::{LogCreateRequest, LogEntry, LogList, LogListQuery, error_response, repository_error};

#[utoipa::path(post, path = "/api/v1/device/log/", tag = "Logs", summary = "Create a structured log entry", description = "Validates and persists one authenticated structured application log, applying legacy defaults for omitted fields.", security(("bearer_auth" = [])), request_body = LogCreateRequest, responses((status = 201, description = "Log entry created", body = LogEntry), (status = 400, description = "Invalid log entry", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Log persistence failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<LogCreateRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    if !matches!(
        request.log_type.as_str(),
        "build" | "test" | "device" | "general"
    ) || !matches!(request.level.as_str(), "info" | "warn" | "error" | "debug")
        || request.reference_id.trim().is_empty()
        || !request.data.is_object()
    {
        return error_response(StatusCode::BAD_REQUEST, "validation", "Invalid log entry");
    }
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.create_log(&request).await {
        Ok(log) => (StatusCode::CREATED, response_headers, Json(log)).into_response(),
        Err(error) => repository_error(error, "Failed to create log entry"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/log/", tag = "Logs", summary = "List structured log entries", description = "Returns bounded, sortable structured logs with legacy level, source, date, and free-text filters.", params(LogListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated log entries", body = LogList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Log query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LogListQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.list_logs(&query, None).await {
        Ok(logs) => (response_headers, Json(logs)).into_response(),
        Err(error) => repository_error(error, "Failed to list logs"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/log/search", tag = "Logs", summary = "Search structured log data", description = "Requires a non-empty query and searches bounded structured message and metadata fields with pagination.", params(LogListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated matching log entries", body = LogList), (status = 400, description = "Search query is required", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Log query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LogListQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(search) = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "Search query is required",
        );
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.list_logs(&query, Some(search)).await {
        Ok(logs) => (response_headers, Json(logs)).into_response(),
        Err(error) => repository_error(error, "Failed to search logs"),
    }
}
