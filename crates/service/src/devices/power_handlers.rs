use std::sync::Arc;

use super::callbacks::callback_client;
use super::*;
use crate::error::ErrorResponse;
use crate::{auth, events::ServerEvent, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

#[utoipa::path(put, path = "/api/v1/device/relay/toggle/{device_id}", tag = "Devices", summary = "Toggle device power", description = "Resolves the device's relay or Gen5 controller power mapping, sends the controller command, then persists and emits the committed power state.", params(("device_id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Updated relay channel or Gen5 power state"), (status = 400, description = "No power mapping exists", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 500, description = "Power-state persistence failed", body = ErrorResponse), (status = 502, description = "Controller rejected the power command", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn toggle_device_power(
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
    let target = match repository.device_power_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
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
    let url = match reqwest::Url::parse(&format!(
        "http://{controller_ip}:{EDGE_CONTROLLER_PORT}/{path}"
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
            if let Err(error) = state.event_publisher.publish(event) {
                tracing::warn!(%error, device_id, "failed to publish device power update");
            }
            if family == "gen5" {
                result["power"] = target.power_port.unwrap_or_default().into();
            }
            (response_headers, Json(result)).into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => repository_error(error, "Failed to save device power state"),
    }
}

pub(super) fn device_power_event(device_id: &str, power: &str) -> ServerEvent {
    ServerEvent {
        event: "device_power_update".to_owned(),
        payload: serde_json::json!({
            "deviceId": device_id,
            "power": power,
            "changedAt": chrono::Utc::now().to_rfc3339(),
        }),
        room: None,
    }
}

#[utoipa::path(get, path = "/api/v1/device/{id}/reboot", tag = "Devices", summary = "Reboot a device", description = "Public compatibility action that resolves the inventory-authorized controller target, requests reboot, and schedules Gen5 recovery tracking when applicable.", params(("id" = String, Path)), responses((status = 200, description = "Device reboot command completed"), (status = 404, description = "Device or power mapping not found", body = ErrorResponse), (status = 500, description = "Reboot target or recovery persistence failed", body = ErrorResponse), (status = 502, description = "Controller rejected the reboot command", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn reboot_device(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.device_reboot_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
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
    let (Some(controller_ip), Some(power_port)) = (target.controller_ip, target.power_port) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "power_mapping_not_found",
            "No power mapping was found for this Gen5 device",
        );
    };
    let url = match reqwest::Url::parse(&format!(
        "http://{controller_ip}:{EDGE_CONTROLLER_PORT}/reboot-device"
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
    match callback_client()
        .post(url)
        .json(&serde_json::json!({"power": power_port}))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => Json(serde_json::json!({
            "success": true, "message": "Device rebooted successfully",
        }))
        .into_response(),
        Ok(response) => {
            tracing::warn!(status = %response.status(), device_id, "controller rejected device reboot");
            error_response(
                StatusCode::BAD_GATEWAY,
                "reboot_failed",
                "Controller rejected the reboot command",
            )
        }
        Err(error) => {
            tracing::warn!(%error, device_id, "controller reboot callback failed");
            error_response(
                StatusCode::BAD_GATEWAY,
                "controller_unreachable",
                "Controller reboot endpoint is unavailable",
            )
        }
    }
}
