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
use uuid::Uuid;

#[utoipa::path(get, path = "/api/v1/device/relay/devices/available", tag = "Devices", summary = "List devices available for relay assignment", description = "Returns paginated devices not occupied by another active relay mapping, filtered to the target relay controller generation when known.", params(AvailableRelayDevicesQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated available devices", body = AvailableRelayDevicesResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Available-device query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn available_relay_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AvailableRelayDevicesQuery>,
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
    match repository.available_relay_devices(&query).await {
        Ok(devices) => (response_headers, Json(devices)).into_response(),
        Err(error) => repository_error(error, "Failed to list available relay devices"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/relay/controller/{id}", tag = "Devices", summary = "List relays for a controller", description = "Returns each non-deleted relay on the controller with ordered channel and attached-device projections.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Controller relay inventory", body = Vec<RelayRecord>), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Relay query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn relays_for_controller(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(controller_id): Path<String>,
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
    match repository.relays_for_controller(&controller_id).await {
        Ok(relays) => (response_headers, Json(relays)).into_response(),
        Err(error) => repository_error(error, "Failed to list controller relays"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/relay/channels/relay/{id}", tag = "Devices", summary = "List channels for a relay", description = "Returns ordered non-deleted channel records for one relay, including attached-device summaries.", params(("id" = Uuid, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Relay channel inventory", body = Vec<RelayChannelRecord>), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay not found", body = ErrorResponse), (status = 500, description = "Relay channel query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn channels_for_relay(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(relay_id): Path<Uuid>,
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
    match repository.channels_for_relay(relay_id).await {
        Ok(Some(channels)) => (response_headers, Json(channels)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Relay not found"),
        Err(error) => repository_error(error, "Failed to list relay channels"),
    }
}
