use std::sync::Arc;

use super::*;
use crate::error::ErrorResponse;
use crate::{auth, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

#[utoipa::path(get, path = "/api/v1/device/{id}/builds", tag = "Devices", summary = "List build versions for a device", description = "Resolves the device type and returns available configured artifact/build version folders.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Build version folders", body = Vec<String>), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 500, description = "Build discovery failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn builds_for_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let folder_name = match repository.artifact_folder_for_device(&device_id).await {
        Ok(Some(folder_name)) => folder_name,
        Ok(None) => return (response_headers, Json(Vec::<String>::new())).into_response(),
        Err(error) => {
            return repository_error(error, "Failed to load device artifact configuration");
        }
    };
    let versions =
        artifacts::discover_versions(&state.config.device.artifacts_base_url, &folder_name).await;
    (response_headers, Json(versions)).into_response()
}

#[utoipa::path(get, path = "/api/v1/device/builds", tag = "Devices", summary = "List releases for a device type", description = "Returns retained release rows matching the requested device type for execution and flashing selection.", params(BuildsQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Matching releases", body = Vec<serde_json::Value>), (status = 400, description = "Device type is missing", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Release query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn builds_for_device_type(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<BuildsQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let device_type = query.device_type.as_deref().unwrap_or("").trim();
    if device_type.is_empty() {
        return (response_headers, Json(Vec::<serde_json::Value>::new())).into_response();
    }
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.builds_for_device_type(device_type).await {
        Ok(builds) => (response_headers, Json(builds)).into_response(),
        Err(error) => repository_error(error, "Failed to list builds"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/config/artifacts", tag = "Devices", summary = "Configure artifact folders", description = "Creates or updates device-type artifact folder metadata used by uploads, scanners, NFS/TFTP staging, and test preparation.", security(("bearer_auth" = [])), request_body = DeviceTypeFolderRequest, responses((status = 200, description = "Artifact folders configured"), (status = 400, description = "Invalid artifact configuration", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Artifact configuration failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn configure_artifacts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<DeviceTypeFolderRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let entries = match request {
        DeviceTypeFolderRequest::One(entry) => vec![entry],
        DeviceTypeFolderRequest::Many(entries) => entries,
    };
    if entries.is_empty()
        || entries.iter().any(|entry| {
            entry.device_type.trim().is_empty()
                || entry.folder_name.trim().is_empty()
                || entry.device_family.trim().is_empty()
                || entry.default_version.trim().is_empty()
        })
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_artifact_configuration",
            "At least one complete artifact configuration is required",
        );
    }
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.configure_artifacts(&entries).await {
        Ok(configured) => (
            response_headers,
            Json(serde_json::json!({
                "message": "Device type folders updated successfully", "data": configured,
            })),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to configure artifacts"),
    }
}
