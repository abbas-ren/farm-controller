use super::super::{error_response, repository_error};
use crate::{auth, events::ServerEvent, state::AppState};
use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use std::sync::Arc;

use super::super::{BuildFlagRequest, BuildStatusRequest, BuildStatusResponse};

const BUILD_STATUSES: [&str; 4] = ["untested", "testing", "passed", "failed"];

#[utoipa::path(put, path = "/api/v1/device/build/{id}/status", tag = "Builds", summary = "Set build validation status", description = "Allows an administrator to set the persisted build lifecycle status, synchronizes the faulty flag, and publishes the committed status event.", params(("id" = String, Path)), security(("bearer_auth" = [])), request_body = BuildStatusRequest, responses((status = 200, description = "Build status updated", body = BuildStatusResponse), (status = 400, description = "Invalid build status", body = crate::error::ErrorResponse), (status = 401, description = "Authentication required", body = crate::error::ErrorResponse), (status = 403, description = "Administrator role required", body = crate::error::ErrorResponse), (status = 404, description = "Build not found", body = crate::error::ErrorResponse), (status = 503, description = "Device persistence unavailable", body = crate::error::ErrorResponse)))]
pub async fn update_build_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
    Json(request): Json<BuildStatusRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let status = request.status.trim().to_lowercase();
    if !BUILD_STATUSES.contains(&status.as_str()) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "status must be untested, testing, passed, or failed",
        );
    }
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.update_build_status(&build_id, &status).await {
        Ok(Some(result)) => {
            publish_build_event(
                &state,
                "build_status_changed",
                serde_json::json!({
                    "releaseId": build_id,
                    "status": status,
                    "isFaulty": result.is_faulty,
                    "version": result.version,
                    "deviceType": result.device_type,
                }),
            );
            publish_alert(&state, result.alert);
            (
                response_headers,
                Json(BuildStatusResponse {
                    id: build_id,
                    status,
                    is_faulty: result.is_faulty,
                }),
            )
                .into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Build not found"),
        Err(error) => repository_error(error, "Failed to update build status"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/build/{id}/flag", tag = "Builds", summary = "Flag or unflag a build", description = "Atomically updates the release fault/status fields, then publishes the committed build event and persisted alert.", params(("id" = String, Path)), security(("bearer_auth" = [])), request_body = BuildFlagRequest, responses((status = 200, description = "Build flag updated", body = super::super::BuildFlagResponse), (status = 400, description = "isFaulty boolean is required", body = crate::error::ErrorResponse), (status = 401, description = "Authentication required", body = crate::error::ErrorResponse), (status = 403, description = "Administrator role required", body = crate::error::ErrorResponse), (status = 500, description = "Build not found or update failed", body = crate::error::ErrorResponse), (status = 503, description = "Device persistence unavailable", body = crate::error::ErrorResponse)))]
pub async fn flag_build(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
    payload: Result<Json<BuildFlagRequest>, JsonRejection>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "validation",
                "isFaulty boolean is required",
            );
        }
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.flag_build(&build_id, &request).await {
        Ok(Some(result)) => {
            publish_build_event(
                &state,
                "build_flagged",
                serde_json::json!({
                    "releaseId": build_id,
                    "isFaulty": request.is_faulty,
                    "version": result.version,
                    "deviceType": result.device_type,
                }),
            );
            publish_alert(&state, result.alert);
            (
                response_headers,
                Json(serde_json::json!({"id": build_id, "isFaulty": request.is_faulty})),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &format!("Release not found: {build_id}"),
        ),
        Err(error) => repository_error(error, "Failed to flag build"),
    }
}

#[utoipa::path(delete, path = "/api/v1/device/build/{id}", tag = "Builds", summary = "Delete a build", description = "Deletes the release, then best-effort removes safe local and controller IPL artifacts before publishing the committed alert/event.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 204, description = "Build deleted"), (status = 401, description = "Authentication required", body = crate::error::ErrorResponse), (status = 403, description = "Administrator role required", body = crate::error::ErrorResponse), (status = 500, description = "Build not found or deletion failed", body = crate::error::ErrorResponse), (status = 503, description = "Device persistence unavailable", body = crate::error::ErrorResponse)))]
pub async fn delete_build(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
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
    match repository.delete_build(&build_id).await {
        Ok(Some(target)) => {
            if safe_segment(&target.folder_name) && safe_segment(&target.version) {
                let path = std::path::Path::new(&state.config.device.build_artifacts_dir)
                    .join(&target.folder_name)
                    .join(&target.version);
                if let Err(error) = tokio::fs::remove_dir_all(path).await
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(%error, family = %target.device_family, "failed to delete build artifacts");
                }
            }
            if let Err(error) =
                crate::workers::remove_build_ipl(&state, &target.device_family, &target.version)
                    .await
            {
                tracing::error!(%error, family = %target.device_family, version = %target.version, "failed to remove build IPL payload");
            }
            publish_build_event(
                &state,
                "build_deleted",
                serde_json::json!({
                    "releaseId": build_id,
                    "version": target.version,
                    "deviceType": target.device_type,
                }),
            );
            publish_alert(&state, target.alert);
            (response_headers, StatusCode::NO_CONTENT).into_response()
        }
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &format!("Release not found: {build_id}"),
        ),
        Err(error) => repository_error(error, "Failed to delete build"),
    }
}

fn publish_build_event(state: &AppState, event: &str, payload: serde_json::Value) {
    if let Err(error) = state.event_publisher.publish(ServerEvent {
        event: event.to_owned(),
        payload,
        room: None,
    }) {
        tracing::warn!(%error, event, "failed to publish build event");
    }
}

fn publish_alert(state: &AppState, alert: serde_json::Value) {
    if let Err(error) = state
        .event_publisher
        .publish(ServerEvent::alert("device-alert", alert))
    {
        tracing::warn!(%error, "failed to publish build alert");
    }
}

fn safe_segment(value: &str) -> bool {
    let mut components = std::path::Path::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}
