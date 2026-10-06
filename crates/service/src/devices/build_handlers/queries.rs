use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::{auth, error::ErrorResponse, state::AppState};

use super::super::{BuildFilters, BuildList, BuildListQuery, error_response, repository_error};

#[utoipa::path(get, path = "/api/v1/device/build/filters", tag = "Builds", summary = "List build filters", description = "Returns deterministic distinct device-family and device-type values from retained release rows.", security(("bearer_auth" = [])), responses((status = 200, description = "Available build filters", body = BuildFilters), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Build filter query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn build_filters(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
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
    match repository.build_filters().await {
        Ok(filters) => (response_headers, Json(filters)).into_response(),
        Err(error) => repository_error(error, "Failed to get build filters"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/build/{id}", tag = "Builds", summary = "Get a build", description = "Returns the complete heterogeneous legacy release row so existing frontend fields remain available.", params(("id" = String, Path, description = "Build ID")), security(("bearer_auth" = [])), responses((status = 200, description = "Complete legacy release row"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Build not found", body = ErrorResponse), (status = 500, description = "Build query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn build_by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
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
    match repository.build_by_id(&build_id).await {
        Ok(Some(build)) => (response_headers, Json(build)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Build not found"),
        Err(error) => repository_error(error, "Failed to get build"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/build", tag = "Builds", summary = "List builds", description = "Returns paginated complete legacy release rows with allowlisted sorting and family, type, version, fault, and search filters.", params(BuildListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated builds", body = BuildList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Build query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list_builds(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<BuildListQuery>,
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
    match repository.list_builds(&query).await {
        Ok(builds) => (response_headers, Json(builds)).into_response(),
        Err(error) => repository_error(error, "Failed to list builds"),
    }
}
