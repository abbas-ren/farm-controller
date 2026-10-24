//! Mark-device-flashing and related IPL upgrade state handlers.
//!
//! Ports Node `DeviceController.setDeviceUpgrading` (`GET /api/v1/device/{id}/flashing`)
//! from `deviceRoutes.ts`. EdgeController / workers mark a DUT as flashing during IPL.

use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    error::ErrorResponse,
    socket::types::{farm::ServerEvent, frontend::FrontendMessagePayload},
    state::AppState,
    workers::types::DeviceState,
};

use super::{
    DeviceFlashingResponse, error_response, repository_error, repository_types::RtosCaptureTarget,
};

const RTOS_START_TIMEOUT: Duration = Duration::from_secs(10);

#[utoipa::path(
    get,
    path = "/api/v1/device/{id}/flashing",
    tag = "Devices",
    summary = "Mark a device flash as started",
    description = "Public device callback. Atomically derives upgrading/flashing flags from the active queue job, emits the committed state, and best-effort starts RTOS capture for test jobs.",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, description = "Flashing state evaluated", body = DeviceFlashingResponse),
        (status = 404, description = "Device not found", body = ErrorResponse),
        (status = 500, description = "Device-state persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
/// PURPOSE: Mark a device flash as started and best-effort begin RTOS capture.
/// USE: called by EdgeController (public callback); publishes Socket.IO frontend events.
pub async fn mark_device_flashing(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::flashing_handlers", "mark_device_flashing");
    let Some(repository) = &state.device_repository else {
        // Guard: device repository is not wired in this process.
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.mark_device_flashing(&device_id).await {
        Ok(Some(result)) => {
            let device = result.device;
            if device.get("state").and_then(serde_json::Value::as_str) == Some("busy") {
                let event = ServerEvent::frontend(FrontendMessagePayload::DeviceStateUpdate {
                    device_id: device_id.clone(),
                    state: DeviceState::from(&device["state"].to_string()),
                    upgrading: device["upgrading"].as_bool().unwrap_or(false),
                    flashing: device["flashing"].as_bool().unwrap_or(false),
                    changed_at: chrono::Utc::now().to_rfc3339(),
                });

                // Publish: notify connected frontend clients via Socket.IO.
                if let Err(error) = state.event_publisher.publish(event) {
                    tracing::warn!(%error, device_id, "failed to publish flashing state update");
                }
            }
            if let Some(target) = result.rtos_target
                && let Err(error) = start_rtos_capture(
                    &target,
                    state.config.device.edge_controller_http_port,
                )
                .await
            {
                tracing::warn!(%error, device_id, "best-effort RTOS capture start failed");
            }
            Json(serde_json::json!({
                "success": true,
                "message": format!("Device {device_id} is now marked as upgrading")
            }))
            .into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "Failed to update device state"),
    }
}

/// PURPOSE: Ask EdgeController to start RTOS UART capture while flashing.
/// USE: mark_device_flashing path.
async fn start_rtos_capture(
    target: &RtosCaptureTarget,
    controller_port: u16,
) -> Result<(), String> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::flashing_handlers", "start_rtos_capture");
    let url = reqwest::Url::parse(&format!(
        "http://{}:{controller_port}/rtos/start",
        target.controller_ip
    ))
    .map_err(|error| error.to_string())?;
    crate::external_http::client(RTOS_START_TIMEOUT)
        .map_err(|error| error.to_string())?
        .post(url)
        .json(&rtos_start_payload(target))
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// PURPOSE: Build the EdgeController payload to start RTOS capture.
/// USE: start_rtos_capture.
fn rtos_start_payload(target: &RtosCaptureTarget) -> serde_json::Value {
    let _api_trace = crate::observability::ApiTrace::enter("devices::flashing_handlers", "rtos_start_payload");
    serde_json::json!({
        "mac": target.mac_address,
        "gen": target.generation,
        "rtos": target.rtos_port,
        "serial": target.relay_serial,
        "channel": target.relay_channel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
/// PURPOSE: Assert Gen4 RTOS start payload keeps serial and channel mapping.
/// USE: regression for EdgeController /rtos/start Gen4 fields.
    fn gen4_rtos_payload_preserves_relay_mapping() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::flashing_handlers", "gen4_rtos_payload_preserves_relay_mapping");
        let payload = rtos_start_payload(&RtosCaptureTarget {
            controller_ip: "192.0.2.1".to_owned(),
            mac_address: "00:11:22:33:44:55".to_owned(),
            generation: 4,
            rtos_port: None,
            relay_serial: Some("relay-1".to_owned()),
            relay_channel: Some(2),
        });
        assert_eq!(payload["gen"], 4);
        assert_eq!(payload["serial"], "relay-1");
        assert_eq!(payload["channel"], 2);
    }
}
