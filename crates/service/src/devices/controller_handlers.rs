//! Admin device/controller mutation handlers.
//!
//! Ports Node `DeviceController` delete/edit routes: `DELETE /api/v1/device/{id}`,
//! `PUT|DELETE /api/v1/device/controller/{id}` from `deviceRoutes.ts`. Frontend admin UI
//! calls these; EdgeController receives outbound delete/devCon notifications.

use std::sync::Arc;

use super::callbacks::callback_client;
use super::*;
use crate::error::ErrorResponse;
use crate::socket::types::farm::ServerEvent;
use crate::socket::types::frontend::{Action, FrontendMessagePayload};
use crate::{auth, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

/// Query parameters for `DELETE /api/v1/device/{id}`.
#[derive(Debug, Default, serde::Deserialize)]
pub struct DeleteDeviceQuery {
    /// When true, skip EdgeAgent acknowledgement and delete anyway.
    force: Option<bool>,
}

/// PURPOSE: Delete a device after optional EdgeAgent acknowledgement (force skips ack).
/// USE: Frontend admin UI; notifies EdgeController/EdgeAgent.
#[utoipa::path(delete, path = "/api/v1/device/{id}", tag = "Devices", summary = "Delete a device", description = "Requests remote device cleanup when reachable, then applies the retained database deletion and committed inventory event behavior. Set force=true to skip EdgeAgent acknowledgement for an offline device.", params(("id" = String, Path), ("force" = Option<bool>, Query, description = "Skip EdgeAgent acknowledgement and remove the persisted device")), security(("bearer_auth" = [])), responses((status = 200, description = "Device deleted"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 500, description = "Device deletion failed", body = ErrorResponse), (status = 502, description = "Device did not acknowledge deletion", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn delete_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Query(query): Query<DeleteDeviceQuery>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::controller_handlers", "delete_device");
    // Auth: require valid bearer token with administrator role.
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        // Guard: device repository is not wired in this process.
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.device_delete_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => return repository_error(error, "Failed to load device"),
    };
    if query.force.unwrap_or(false) {
        tracing::warn!(
            device_id,
            "forced device deletion is skipping EdgeAgent acknowledgement"
        );
    } else {
        let url = match reqwest::Url::parse(&format!(
            "http://{}:{EDGE_CONTROLLER_PORT}/delete",
            target.ip_address
        )) {
            Ok(url) => url,
            Err(_) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_device_address",
                    "Device IP address is invalid",
                );
            }
        };
        // Edge: outbound HTTP call to EdgeController.
        match callback_client()
            .post(url)
            .json(&serde_json::json!({"UID": device_id}))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                tracing::warn!(status = %response.status(), device_id, "device deletion was rejected");
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    "device_callback_failed",
                    "Device did not acknowledge deletion",
                );
            }
            Err(error) => {
                tracing::warn!(%error, device_id, "device deletion callback failed");
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    "device_unreachable",
                    "Device delete endpoint is unavailable",
                );
            }
        }
    }
    let normalized_family = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .replace(' ', "")
        .to_lowercase();
    if matches!(normalized_family.as_str(), "gen4" | "gen5")
        && let (Some(controller_id), Some(controller_ip)) =
            (&target.controller_id, &target.controller_ip)
    {
        let generation = normalized_family
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u32>()
            .unwrap_or(0);
        if let Ok(url) = reqwest::Url::parse(&format!(
            "http://{controller_ip}:{EDGE_CONTROLLER_PORT}/devCon/delete"
        )) {
            let mut payload =
                serde_json::json!({"uid": controller_id, "gen": generation, "mac": device_id});
            if normalized_family == "gen4" {
                if let Some(serial) = &target.relay_serial {
                    payload["serial"] = serial.clone().into();
                }
                if let Some(channel) = target.relay_channel {
                    payload["channel"] = channel.into();
                }
            }
            // Edge: outbound HTTP call to EdgeController.
            if let Err(error) = callback_client().post(url).json(&payload).send().await {
                tracing::warn!(%error, device_id, "controller device-delete notification failed; continuing");
            }
        }
    }
    match repository.delete_device(&device_id).await {
        Ok(true) => (response_headers, Json(serde_json::json!({"success": true, "message": format!("Device {device_id} successfully deleted")}))).into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "Failed to delete device"),
    }
}

/// PURPOSE: Edit a device controller
/// USE: Frontend admin UI (bearer + admin role).
#[utoipa::path(put, path = "/api/v1/device/controller/{id}", tag = "Devices", summary = "Edit a device controller", description = "Updates mutable controller inventory fields and emits the committed controller-changed event.", params(("id" = String, Path)), security(("bearer_auth" = [])), request_body = ControllerEditRequest, responses((status = 200, description = "Controller updated"), (status = 400, description = "Invalid controller update", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Controller not found", body = ErrorResponse), (status = 500, description = "Controller update failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn edit_controller(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(controller_id): Path<String>,
    Json(request): Json<ControllerEditRequest>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::controller_handlers", "edit_controller");
    // Auth: require valid bearer token with administrator role.
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    if request
        .ip_address
        .as_deref()
        .is_some_and(|address| address.trim().is_empty())
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_address",
            "Controller IP address must not be empty",
        );
    }
    let Some(repository) = state.device_repository.clone() else {
        // Guard: device repository is not wired in this process.
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.edit_controller(&controller_id, &request).await {
        Ok(Some(controller)) => {
            publish_controller_changed(&state, "updated", &controller_id);
            (
                response_headers,
                Json(serde_json::json!({"success": true, "data": controller})),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Device controller not found",
        ),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "Failed to update device controller"),
    }
}

/// PURPOSE: Delete a device controller
/// USE: Frontend admin UI (bearer + admin role); talks to EdgeController over HTTP.
#[utoipa::path(delete, path = "/api/v1/device/controller/{id}", tag = "Devices", summary = "Delete a device controller", description = "Deletes the controller inventory relationship and emits the committed controller-changed event.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 204, description = "Controller deleted"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Controller not found", body = ErrorResponse), (status = 500, description = "Controller deletion failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn delete_controller(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(controller_id): Path<String>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::controller_handlers", "delete_controller");
    // Auth: require valid bearer token with administrator role.
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        // Guard: device repository is not wired in this process.
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.controller_delete_target(&controller_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Device controller not found",
            );
        }
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => return repository_error(error, "Failed to load device controller"),
    };
    let generation = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse::<u32>()
        .unwrap_or(0);
    // Edge: outbound HTTP call to EdgeController.
    if let Ok(url) = reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/devCon/delete",
        target.ip_address
    )) && let Err(error) = callback_client()
        .post(url)
        .json(&serde_json::json!({"uid": controller_id, "gen": generation}))
        .send()
        .await
    {
        tracing::warn!(%error, controller_id, "controller delete notification failed; continuing");
    }
    match repository.delete_controller(&controller_id).await {
        Ok(true) => {
            publish_controller_changed(&state, "deleted", &controller_id);
            (response_headers, StatusCode::NO_CONTENT).into_response()
        }
        Ok(false) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Device controller not found",
        ),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "Failed to delete device controller"),
    }
}

/// PURPOSE: Publish DeviceControllerChanged so the frontend refreshes controller inventory.
/// USE: register/edit/delete controller success paths.
pub(super) fn publish_controller_changed(state: &AppState, action: &str, controller_id: &str) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::controller_handlers", "publish_controller_changed");
    // Publish: notify connected frontend clients via Socket.IO.
    if let Err(error) = state.event_publisher.publish(ServerEvent::frontend(
        FrontendMessagePayload::DeviceControllerChanged {
            action: Action::from(action),
            controller_id: controller_id.to_string(),
        },
    )) {
        tracing::warn!(%error, action, controller_id, "failed to publish controller change");
    }
}
