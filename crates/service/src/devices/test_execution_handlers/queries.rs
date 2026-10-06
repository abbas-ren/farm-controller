//! Execution list, detail, and result query handlers.

use super::*;
use crate::{auth, error::ErrorResponse, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;

#[utoipa::path(get, path = "/api/v1/device/test/execution", tag = "Tests", summary = "List test executions", description = "Returns current-user executions with bounded pagination, device-type search, allowlisted sorting, and computed duration/result counts.", params(ExecutionListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated executions", body = ExecutionList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ExecutionListQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let user_id = auth::token_subject(&headers).unwrap_or_else(|| "system".to_owned());
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.list_test_executions(&user_id, &query).await {
        Ok(executions) => (response_headers, Json(executions)).into_response(),
        Err(error) => repository_error(error, "Failed to list test executions"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/device/{id}", tag = "Tests", summary = "Get latest execution for a device", description = "Returns the current user's newest execution for the device and attaches configured JSON logs when present.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Complete legacy execution row with logs", body = Object), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Execution not found", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn by_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let user_id = auth::token_subject(&headers).unwrap_or_else(|| "system".to_owned());
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository
        .test_execution_by_device(&user_id, &device_id)
        .await
    {
        Ok(Some(mut execution)) => {
            super::case_queries::add_logs(&state, &mut execution).await;
            (response_headers, Json(execution)).into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test execution"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/progress/list", tag = "Tests", summary = "List active executions", description = "Returns current-user executions in queued, not-executed, or in-progress states for frontend progress tracking.", security(("bearer_auth" = [])), responses((status = 200, description = "Current-user active executions", body = [ActiveExecutionRecord]), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn progress(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let user_id = auth::token_subject(&headers).unwrap_or_else(|| "system".to_owned());
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.in_progress_test_executions(&user_id).await {
        Ok(executions) => (response_headers, Json(executions)).into_response(),
        Err(error) => repository_error(error, "Failed to list test executions"),
    }
}

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct ExecutionDetailQuery {
    pub table: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/{id}", tag = "Tests", summary = "Get a test execution", description = "Returns current-user detail for an ID or latest active execution; table mode preserves the historically unscoped summary and attaches configured logs.", params(("id" = String, Path), ExecutionDetailQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Complete legacy execution row or table summary with logs", body = Object), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Test execution not found", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn execution(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<ExecutionDetailQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let user_id = auth::token_subject(&headers).unwrap_or_else(|| "system".to_owned());
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let result = if query.table.is_some() {
        repository.test_execution_summary(&user_id, &id).await
    } else {
        repository.test_execution(&user_id, &id).await
    };
    match result {
        Ok(Some(mut execution)) => {
            super::case_queries::add_logs(&state, &mut execution).await;
            (response_headers, Json(execution)).into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test execution"),
    }
}

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct BuildExecutionQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/build/{buildId}", tag = "Tests", summary = "List executions for a build", description = "Returns bounded execution summaries for one build using legacy limit/offset pagination.", params(("buildId" = String, Path), BuildExecutionQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated build executions", body = BuildExecutionList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn by_build(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
    Query(query): Query<BuildExecutionQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let limit = query.limit.unwrap_or(5).max(1);
    let offset = query.offset.unwrap_or(0).max(0);
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository
        .executions_by_build(&build_id, limit, offset)
        .await
    {
        Ok(executions) => (response_headers, Json(executions)).into_response(),
        Err(error) => repository_error(error, "Failed to get test executions by build"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/results/{id}", tag = "Tests", summary = "Get execution results", description = "Returns execution status together with complete ordered legacy testcase rows for result inspection.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Execution status and complete legacy testcase rows", body = Object), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Execution not found", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn results(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
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
    match repository.test_results(&test_id).await {
        Ok(Some(results)) => (response_headers, Json(results)).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test results"),
    }
}
