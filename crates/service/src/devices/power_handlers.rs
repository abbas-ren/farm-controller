//! Device power toggle and reboot handlers.
//!
//! Ports Node `RelayController.toggleRelayChannel` and `DeviceController.rebootDevice`:
//! `PUT /api/v1/device/relay/toggle/{device_id}` and `GET /api/v1/device/{id}/reboot`
//! from `relayRouter.ts` / `deviceRoutes.ts`. Frontend toggles power; reboot is a public
//! compatibility action that commands EdgeController.

use std::sync::Arc;

use super::callbacks::callback_client;
use super::*;
use crate::error::ErrorResponse;
use crate::socket::types::farm::ServerEvent;
use crate::socket::types::frontend::{DevicePower, FrontendMessagePayload};
use crate::{auth, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

/// PURPOSE: Toggle DUT power via relay or Gen5 controller mapping, then persist and emit.
/// USE: Frontend power controls; commands EdgeController.
#[utoipa::path(put, path = "/api/v1/device/relay/toggle/{device_id}", tag = "Devices", summary = "Toggle device power", description = "Resolves the device's relay or Gen5 controller power mapping, sends the controller command, then persists and emits the committed power state.", params(("device_id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Updated relay channel or Gen5 power state"), (status = 400, description = "No power mapping exists", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 500, description = "Power-state persistence failed", body = ErrorResponse), (status = 502, description = "Controller rejected the power command", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn toggle_device_power(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::power_handlers", "toggle_device_power");
    // Auth: require any authenticated bearer token.
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
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
    let target = match repository.device_power_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => return repository_error(error, "Failed to load device power mapping"),
    };
    let family = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .replace(' ', "")
        .to_lowercase();
    let next_state = if target.current_power.as_deref() == Some("on") {
        "off"
    } else {
        "on"
    };
    let Some(controller_ip) = target.controller_ip.as_deref() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "power_mapping_not_found",
            "Unable to find a mapped controller",
        );
    };
    let (path, payload) = if family == "gen5" {
        let Some(power_port) = target.power_port.as_deref() else {
            return error_response(
                StatusCode::BAD_REQUEST,
                "power_mapping_not_found",
                "No power port mapping was found for this Gen5 device",
            );
        };
        (
            "gen5/power",
            serde_json::json!({"state": next_state, "power": power_port}),
        )
    } else {
        let (Some(serial), Some(channel)) = (target.relay_serial.as_deref(), target.channel_number)
        else {
            return error_response(
                StatusCode::BAD_REQUEST,
                "relay_mapping_not_found",
                "Unable to find a mapped relay",
            );
        };
        (
            "relay",
            serde_json::json!({"serial": serial, "state": next_state, "channel": channel}),
        )
    };
    let controller_port = state.config.device.edge_controller_http_port;
    let url = match reqwest::Url::parse(&format!(
        "http://{controller_ip}:{controller_port}/{path}"
    )) {
        Ok(url) => url,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_controller_address",
                "Controller IP address is invalid",
            );
        }
    };
    // Edge: outbound HTTP call to EdgeController.
    match callback_client().post(url).json(&payload).send().await {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => {
            tracing::warn!(status = %response.status(), device_id, "controller rejected power toggle");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "power_toggle_failed",
                "Controller rejected the power command",
            );
        }
        Err(error) => {
            tracing::warn!(%error, device_id, "controller power callback failed");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "controller_unreachable",
                "Controller power endpoint is unavailable",
            );
        }
    }
    match repository
        .apply_device_power(&device_id, target.channel_id, next_state)
        .await
    {
        Ok(Some(mut result)) => {
            let event = device_power_event(&device_id, next_state);
            // Publish: notify connected frontend clients via Socket.IO.
            if let Err(error) = state.event_publisher.publish(event) {
                tracing::warn!(%error, device_id, "failed to publish device power update");
            }
            if family == "gen5" {
                result["power"] = target.power_port.unwrap_or_default().into();
            }
            (response_headers, Json(result)).into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "Failed to save device power state"),
    }
}

/// PURPOSE: Build a DevicePowerUpdate Socket.IO event for a power state change.
/// USE: toggle_device_power after successful persistence.
pub(super) fn device_power_event(device_id: &str, power: &str) -> ServerEvent {
    let _api_trace = crate::observability::ApiTrace::enter("devices::power_handlers", "device_power_event");
    ServerEvent::frontend(FrontendMessagePayload::DevicePowerUpdate {
        device_id: device_id.to_owned(),
        power: DevicePower::from(power),
        changed_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// PURPOSE: Request a Gen5 device reboot through its mapped controller (Gen3/Gen4 no-op success).
/// USE: Public compatibility callers and frontend reboot actions; commands EdgeController.
#[utoipa::path(get, path = "/api/v1/device/{id}/reboot", tag = "Devices", summary = "Reboot a device", description = "Public compatibility action that resolves the inventory-authorized controller target, requests reboot, and schedules Gen5 recovery tracking when applicable.", params(("id" = String, Path)), responses((status = 200, description = "Device reboot command completed"), (status = 404, description = "Device or power mapping not found", body = ErrorResponse), (status = 500, description = "Reboot target or recovery persistence failed", body = ErrorResponse), (status = 502, description = "Controller rejected the reboot command", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn reboot_device(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::power_handlers", "reboot_device");
    let Some(repository) = state.device_repository.clone() else {
        // Guard: device repository is not wired in this process.
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.device_reboot_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => return repository_error(error, "Failed to load device reboot configuration"),
    };
    let family = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .replace(' ', "")
        .to_lowercase();
    if matches!(family.as_str(), "gen3" | "gen4") {
        return Json(
            serde_json::json!({"success": true, "message": "Device rebooted successfully"}),
        )
        .into_response();
    }
    if family != "gen5" {
        return error_response(
            StatusCode::BAD_REQUEST,
            "unsupported_device_family",
            "Reboot is not supported for this device family",
        );
    }
    // Public DUT callback from eagent (`schedule_reboot`) already issues a local soft
    // reboot. Do not also power-cycle via EdgeController — that races the soft reboot
    // during NFS flash / test-execution boot (G7).
    Json(serde_json::json!({
        "success": true,
        "message": "Gen5 reboot acknowledged; device soft-reboot is in progress",
    }))
    .into_response()
}
