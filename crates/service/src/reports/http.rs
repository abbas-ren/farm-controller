//! Axum routes and OpenAPI handlers for report operations.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use uuid::Uuid;

use crate::{error::ErrorResponse, state::AppState};

use super::{
    ConfluenceError, ConfluenceUploadRequest, ReportAcknowledgement, ReportError,
    ReportJobSnapshot, ReportRequest, constants::http as messages,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/report", post(enqueue_report))
        .route("/report/{job_id}", get(report_status))
        .route("/upload-confluence", post(upload_confluence))
}

#[utoipa::path(post, path = "/report", tag = "Reports", summary = "Queue report generation", description = "Validates report identifiers and enqueues native generation on the bounded in-process worker.", request_body = ReportRequest,
    responses((status = 202, description = "Report queued", body = ReportAcknowledgement), (status = 400, description = "Invalid request", body = ErrorResponse), (status = 500, description = "Report initialization failed", body = ErrorResponse), (status = 503, description = "Reports or queue unavailable", body = ErrorResponse)))]
pub async fn enqueue_report(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ReportRequest>,
) -> axum::response::Response {
    let Some(service) = &state.reports else {
        return report_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            messages::REPORTS_UNAVAILABLE,
        );
    };
    match service.enqueue(request.clone()).await {
        Ok(job_id) => (
            StatusCode::ACCEPTED,
            Json(ReportAcknowledgement {
                status: "acknowledged",
                job_id,
                test_id: request.test_id,
                build_version: request.build_version,
                device_type: request.device_type,
                message: "Report generation queued",
            }),
        )
            .into_response(),
        Err(ReportError::Validation(message)) => {
            report_error(StatusCode::BAD_REQUEST, "validation", &message)
        }
        Err(ReportError::QueueFull | ReportError::WorkerUnavailable) => report_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "queue_unavailable",
            messages::QUEUE_UNAVAILABLE,
        ),
        Err(error) => {
            tracing::error!(%error, "failed to enqueue report");
            report_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                messages::QUEUE_FAILED,
            )
        }
    }
}

#[utoipa::path(get, path = "/report/{job_id}", tag = "Reports", summary = "Get report job status", description = "Returns the current in-process report job snapshot; completed artifacts are referenced by the execution-report APIs.", params(("job_id" = Uuid, Path)),
    responses((status = 200, description = "Report job", body = ReportJobSnapshot), (status = 404, description = "Job not found", body = ErrorResponse), (status = 503, description = "Reports unavailable", body = ErrorResponse)))]
pub async fn report_status(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<Uuid>,
) -> axum::response::Response {
    let Some(service) = &state.reports else {
        return report_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            messages::REPORTS_UNAVAILABLE,
        );
    };
    match service.status(job_id).await {
        Some(status) => Json(status).into_response(),
        None => report_error(StatusCode::NOT_FOUND, "not_found", "Job not found"),
    }
}

#[utoipa::path(post, path = "/upload-confluence", tag = "Reports", summary = "Upload generated report to Confluence", description = "Validates a root-contained generated report, creates or updates its Confluence page, and uploads referenced image attachments.", request_body = ConfluenceUploadRequest,
    responses((status = 202, description = "Report uploaded", body = super::ConfluenceUploadResponse), (status = 400, description = "Invalid request or report missing", body = ErrorResponse), (status = 500, description = "Confluence configuration or artifact access failed", body = ErrorResponse), (status = 502, description = "Confluence request failed", body = ErrorResponse), (status = 503, description = "Reports unavailable", body = ErrorResponse)))]
pub async fn upload_confluence(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfluenceUploadRequest>,
) -> axum::response::Response {
    let Some(service) = &state.reports else {
        return report_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            messages::REPORTS_UNAVAILABLE,
        );
    };
    match service.upload_confluence(request).await {
        Ok(response) => (StatusCode::ACCEPTED, Json(response)).into_response(),
        Err(ConfluenceError::Validation(message)) => {
            report_error(StatusCode::BAD_REQUEST, "validation", &message)
        }
        Err(ConfluenceError::Configuration(message)) => {
            report_error(StatusCode::INTERNAL_SERVER_ERROR, "configuration", &message)
        }
        Err(ConfluenceError::Upstream(message)) => {
            report_error(StatusCode::BAD_GATEWAY, "confluence_upstream", &message)
        }
        Err(ConfluenceError::Io(error)) => {
            tracing::error!(%error, "Confluence artifact access failed");
            report_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "artifact_io",
                messages::CONFLUENCE_ARTIFACT_FAILED,
            )
        }
    }
}

fn report_error(status: StatusCode, code: &str, message: &str) -> axum::response::Response {
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
