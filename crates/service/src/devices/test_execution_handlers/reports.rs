//! Execution report enqueue, status, upload, and download handlers.

use super::*;
use crate::reports::{
    ConfluenceUploadRequest, ConfluenceUploadResponse, ReportRequest, ReportStatus,
};
use crate::{auth, error::ErrorResponse, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use std::sync::Arc;

#[utoipa::path(get, path = "/api/v1/device/test/execution/report/{id}", tag = "Tests", summary = "Get execution report metadata", description = "Returns the newest durable execution-report status row for the requested test execution.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Execution report metadata", body = ExecutionReportRecord), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Report not found", body = ErrorResponse), (status = 500, description = "Report query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn report(
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
    match repository.execution_report(&test_id).await {
        Ok(Some(report)) => (response_headers, Json(report)).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "No report found for this execution",
        ),
        Err(error) => repository_error(error, "Failed to get execution report"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/report/{id}/html", tag = "Tests", summary = "Download execution report HTML", description = "Resolves validated device/build/test path segments beneath the configured results root and returns the generated report HTML.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Execution report HTML", body = String, content_type = "text/html"), (status = 400, description = "Invalid report path", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Execution or HTML not found", body = ErrorResponse), (status = 500, description = "Report lookup or file read failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn report_html(
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
    let target = match repository.execution_report_target(&test_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Test execution not found",
            );
        }
        Err(error) => return repository_error(error, "Failed to get execution report HTML"),
    };
    if !super::validation::safe_segment(&target.0)
        || !super::validation::safe_segment(&target.1)
        || !super::validation::safe_segment(&test_id)
    {
        return error_response(StatusCode::BAD_REQUEST, "validation", "Invalid report path");
    }
    let path = std::path::Path::new(&state.config.reports.test_results_dir)
        .join(target.0)
        .join(target.1)
        .join(&test_id)
        .join("report.html");
    match tokio::fs::read_to_string(path).await {
        Ok(html) => {
            let mut response = (response_headers, html).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            response
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Report HTML not found. Generate the report first.",
        ),
        Err(error) => {
            tracing::error!(%error, "failed to read report HTML");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Failed to get execution report HTML",
            )
        }
    }
}

#[utoipa::path(put, path = "/api/v1/device/test/execution/report/{id}", tag = "Tests", summary = "Queue execution report generation", description = "Creates durable report metadata before enqueueing the bounded native report worker.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Report generation queued", body = ExecutionReportRecord), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Execution missing or incomplete", body = ErrorResponse), (status = 409, description = "Report already active", body = ErrorResponse), (status = 500, description = "Report persistence failed", body = ErrorResponse), (status = 503, description = "Device persistence or report queue unavailable", body = ErrorResponse)))]
pub async fn create_report(
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
    let Some(reports) = &state.reports else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            "Reports are unavailable",
        );
    };
    let target = match repository.report_generation_target(&test_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Test Execution Not Found or not completed yet",
            );
        }
        Err(error) => return repository_error(error, "Failed to create execution report"),
    };
    let job_id = uuid::Uuid::new_v4();
    let report = match repository
        .create_execution_report_record(job_id, &test_id, &target.0, &target.1, &user_id)
        .await
    {
        Ok(report) => report,
        Err(error) => return repository_error(error, "Failed to create execution report"),
    };
    if let Err(error) = reports
        .enqueue_with_id(
            job_id,
            ReportRequest {
                test_id: test_id.clone(),
                build_version: target.1.clone(),
                device_type: target.0.clone(),
                previous_test_id: None,
                previous_build_version: None,
                created_by: Some(user_id.clone()),
            },
        )
        .await
    {
        tracing::error!(%error, "failed to queue execution report");
        let message = error.to_string();
        let _ = repository
            .update_execution_report_status(
                job_id,
                super::validation::REPORT_STATUS_FAILED,
                Some(&message),
            )
            .await;
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            "Report queue is unavailable",
        );
    }
    (response_headers, Json(report)).into_response()
}

#[utoipa::path(put, path = "/api/v1/device/test/execution/report/{id}/upload", tag = "Tests", summary = "Upload an execution report to Confluence", description = "Uploads a completed native report and persists uploaded or failed status before publishing room events.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 202, description = "Report uploaded to Confluence", body = ConfluenceUploadResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Completed report missing", body = ErrorResponse), (status = 500, description = "Report persistence failed", body = ErrorResponse), (status = 502, description = "Confluence upload failed", body = ErrorResponse), (status = 503, description = "Device persistence or reports unavailable", body = ErrorResponse)))]
pub async fn upload_report(
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
    let Some(reports) = &state.reports else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            "Reports are unavailable",
        );
    };
    let report = match repository.execution_report(&test_id).await {
        Ok(Some(report)) => report,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "No completed report found for this test execution. Generate the report first.",
            );
        }
        Err(error) => return repository_error(error, "Failed to upload execution report"),
    };
    let Some(report_id) = report
        .get("id")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
    else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Invalid execution report record",
        );
    };
    let completed = reports
        .status(report_id)
        .await
        .is_some_and(|job| job.status == ReportStatus::Completed)
        || matches!(
            report.get("status").and_then(serde_json::Value::as_str),
            Some(
                super::validation::REPORT_STATUS_COMPLETED
                    | super::validation::REPORT_STATUS_UPLOADED
            )
        );
    if !completed {
        return error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "No completed report found for this test execution. Generate the report first.",
        );
    }
    let device_type = report
        .get("deviceType")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let build_version = report
        .get("buildVersion")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if let Err(error) = repository
        .update_execution_report_status(report_id, super::validation::REPORT_STATUS_UPLOADING, None)
        .await
    {
        return repository_error(error, "Failed to update execution report");
    }
    publish_report_update(
        &state,
        report_id,
        &test_id,
        &user_id,
        super::validation::REPORT_STATUS_UPLOADING,
        None,
        None,
    );
    match reports
        .upload_confluence(ConfluenceUploadRequest {
            test_id: test_id.clone(),
            build_version: build_version.to_owned(),
            device_type: device_type.to_owned(),
        })
        .await
    {
        Ok(result) => {
            if let Err(error) = repository
                .update_execution_report_status(
                    report_id,
                    super::validation::REPORT_STATUS_UPLOADED,
                    None,
                )
                .await
            {
                return repository_error(error, "Failed to update execution report");
            }
            publish_report_update(
                &state,
                report_id,
                &test_id,
                &user_id,
                super::validation::REPORT_STATUS_UPLOADED,
                None,
                None,
            );
            (StatusCode::ACCEPTED, response_headers, Json(result)).into_response()
        }
        Err(error) => {
            let message = error.to_string();
            if let Err(status_error) = repository
                .update_execution_report_status(
                    report_id,
                    super::validation::REPORT_STATUS_FAILED,
                    Some(&message),
                )
                .await
            {
                tracing::error!(%status_error, %report_id, "failed to persist report upload failure");
            } else {
                publish_report_update(
                    &state,
                    report_id,
                    &test_id,
                    &user_id,
                    super::validation::REPORT_STATUS_FAILED,
                    Some(&format!("Confluence upload failed: {message}")),
                    Some(&message),
                );
            }
            tracing::error!(%error, "Confluence upload failed");
            error_response(
                StatusCode::BAD_GATEWAY,
                "confluence_error",
                "Failed to upload execution report",
            )
        }
    }
}

fn publish_report_update(
    state: &AppState,
    report_id: uuid::Uuid,
    test_id: &str,
    user_id: &str,
    status: &str,
    error: Option<&str>,
    upload_error: Option<&str>,
) {
    for event in super::events::report_update_events(
        report_id,
        test_id,
        user_id,
        status,
        error,
        upload_error,
    ) {
        if let Err(publish_error) = state.event_publisher.publish(event) {
            tracing::warn!(%publish_error, %report_id, "failed to publish report update");
        }
    }
}
