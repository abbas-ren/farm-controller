use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{error::ErrorResponse, events::ServerEvent, state::AppState};

use super::{
    DeviceFlashingResponse, EDGE_CONTROLLER_PORT, error_response, repository_error,
    repository_types::RtosCaptureTarget,
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
pub async fn mark_device_flashing(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let Some(repository) = &state.device_repository else {
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
                let event = ServerEvent {
                    event: "device_state_update".to_owned(),
                    payload: serde_json::json!({
                        "deviceId": device_id,
                        "state": device["state"],
                        "upgrading": device["upgrading"],
                        "flashing": device["flashing"],
                        "changedAt": chrono::Utc::now().to_rfc3339(),
                    }),
                    room: None,
                };
                if let Err(error) = state.event_publisher.publish(event) {
                    tracing::warn!(%error, device_id, "failed to publish flashing state update");
                }
            }
            if let Some(target) = result.rtos_target
                && let Err(error) = start_rtos_capture(&target).await
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
        Err(error) => repository_error(error, "Failed to update device state"),
    }
}

async fn start_rtos_capture(target: &RtosCaptureTarget) -> Result<(), String> {
    let url = reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/rtos/start",
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

fn rtos_start_payload(target: &RtosCaptureTarget) -> serde_json::Value {
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
    fn gen4_rtos_payload_preserves_relay_mapping() {
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
