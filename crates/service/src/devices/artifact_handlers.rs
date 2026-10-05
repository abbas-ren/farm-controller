use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::{auth, error::ErrorResponse, state::AppState};

use super::{
    DefaultArtifactCopyQuery, DefaultArtifactCopyResponse, artifacts, error_response,
    repository_error,
};

#[utoipa::path(put, path = "/api/v1/device/config/artifacts/default", tag = "Devices", summary = "Copy default device artifacts", description = "Downloads configured default archives with explicit limits, validates contained paths, and installs safe files into the NFS/TFTP artifact roots.", params(DefaultArtifactCopyQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Default artifacts copied", body = DefaultArtifactCopyResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Artifact download or installation failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn copy_default_artifacts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DefaultArtifactCopyQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
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
    let entries = match repository.artifact_folders().await {
        Ok(entries) => entries,
        Err(error) => {
            return repository_error(error, "Failed to load default artifact configuration");
        }
    };
    if let Err(error) = artifacts::copy_defaults(&state.config.device, &entries, query.force).await
    {
        tracing::error!(%error, "default artifact copy failed");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "artifact_copy_failed",
            "Failed to copy default artifacts",
        );
    }
    (
        response_headers,
        Json(DefaultArtifactCopyResponse {
            success: true,
            message: "Default artifacts copied successfully",
        }),
    )
        .into_response()
}
