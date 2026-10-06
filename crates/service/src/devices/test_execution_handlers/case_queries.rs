//! Execution case, testcase detail, and log query handlers.

use super::*;
use crate::{auth, error::ErrorResponse, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use std::sync::Arc;

pub(super) async fn add_logs(state: &AppState, execution: &mut serde_json::Value) {
    let test_id = execution
        .get("testId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let path =
        std::path::Path::new(&state.config.tests.test_logs_dir).join(format!("{test_id}.json"));
    execution["logs"] = tokio::fs::read(path)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!([]));
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/single/{testId}", tag = "Tests", summary = "Get one execution with cases", description = "Returns one unscoped complete legacy execution row with its ordered testcase rows.", params(("testId" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Complete legacy execution row with testCases", body = Object), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Execution not found", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn single(
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
    match repository.single_test_execution(&test_id).await {
        Ok(Some(execution)) => (response_headers, Json(execution)).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test execution"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/list/{id}", tag = "Tests", summary = "List testcase IDs for an execution", description = "Public device compatibility route returning ordered testcase IDs only while the execution is active and not failed or cancelled.", params(("id" = String, Path)), responses((status = 200, description = "Testcase IDs", body = Vec<String>), (status = 404, description = "Execution not found", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn case_ids(
    State(state): State<Arc<AppState>>,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.execution_case_ids(&test_id).await {
        Ok(Some(ids)) => Json(ids).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test execution"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/cases/{id}", tag = "Tests", summary = "List execution testcase results", description = "Returns ordered testcase result projections only when the execution belongs to the current user; missing ownership preserves the legacy 500.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Execution testcases", body = [ExecutionCaseRecord]), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Execution missing or lookup failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn cases(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
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
    match repository.execution_cases(&user_id, &test_id).await {
        Ok(Some(cases)) => (response_headers, Json(cases)).into_response(),
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Failed to get test cases for execution: Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test cases for execution"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/testcase/{testCaseId}", tag = "Tests", summary = "Get testcase detail", description = "Returns the complete unscoped legacy testcase row; a missing row intentionally preserves the historical 500 response.", params(("testCaseId" = i64, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Complete legacy testcase row", body = Object), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Testcase missing or lookup failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn testcase(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(case_id): Path<i64>,
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
    match repository.test_case(case_id).await {
        Ok(Some(case)) => (response_headers, Json(case)).into_response(),
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Failed to get test case: Test Case Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test case"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/logs/{id}", tag = "Tests", summary = "Download execution logs", description = "Reads configured JSON log entries and returns newline-delimited text; missing or invalid files intentionally produce an empty attachment.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Newline-delimited test logs; missing log files produce an empty download", body = String, content_type = "text/plain", headers(("Content-Disposition" = String, description = "Attachment filename test-{id}-logs.txt"))), (status = 401, description = "Authentication required", body = ErrorResponse)))]
pub async fn logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let path =
        std::path::Path::new(&state.config.tests.test_logs_dir).join(format!("{test_id}.json"));
    let logs = tokio::fs::read(path)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<serde_json::Value>>(&bytes).ok())
        .unwrap_or_default();
    let text = logs
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut response = (response_headers, text).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    if let Ok(value) =
        HeaderValue::from_str(&format!("attachment; filename=\"test-{test_id}-logs.txt\""))
    {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

#[utoipa::path(get, path = "/api/v1/device/test/testcase/{testCaseId}/log", tag = "Tests", summary = "Download a testcase log", description = "Canonicalizes the stored testcase output path beneath the configured results root and serves it with a sanitized attachment filename.", params(("testCaseId" = i64, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Testcase log", body = String, content_type = "text/plain", headers(("Content-Disposition" = String, description = "Attachment filename derived from the stored log"))), (status = 400, description = "Stored log path escapes the configured root", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Log not found", body = ErrorResponse), (status = 500, description = "Log lookup or file read failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn testcase_log(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(case_id): Path<i64>,
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
    let stored = match repository.test_case_log_path(case_id).await {
        Ok(Some(path)) => path,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "No log file associated with this test case",
            );
        }
        Err(error) => return repository_error(error, "Failed to download test case log"),
    };
    let root = std::path::Path::new(&state.config.reports.test_results_dir);
    let candidate = std::path::Path::new(&stored);
    let path = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let Ok(root) = tokio::fs::canonicalize(root).await else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "Log file not found");
    };
    let Ok(path) = tokio::fs::canonicalize(path).await else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "Log file not found");
    };
    if !path.starts_with(&root) {
        return error_response(StatusCode::BAD_REQUEST, "validation", "Invalid log path");
    }
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => {
            let mut response = (response_headers, content).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            );
            if let Some(name) = path.file_name().and_then(|name| name.to_str())
                && let Ok(value) =
                    HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
            {
                response
                    .headers_mut()
                    .insert(header::CONTENT_DISPOSITION, value);
            }
            response
        }
        Err(error) => {
            tracing::error!(%error, "failed to read testcase log");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Failed to download test case log",
            )
        }
    }
}
