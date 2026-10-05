use std::sync::Arc;

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{error::ErrorResponse, events::ServerEvent, state::AppState};

use super::{DeviceRegistration, DeviceRegistrationResponse, error_response, repository_error};

#[utoipa::path(
    post,
    path = "/api/v1/device/",
    tag = "Devices",
    summary = "Register or refresh a device",
    description = "Public EdgeController compatibility endpoint. The MAC address becomes the normalized device ID; repeated registrations update the existing device and synchronize interfaces.",
    request_body = DeviceRegistration,
    responses(
        (status = 201, description = "Device registered or refreshed", body = DeviceRegistrationResponse),
        (status = 400, description = "Invalid device registration", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn register_device(
    State(state): State<Arc<AppState>>,
    payload: Result<Json<DeviceRegistration>, JsonRejection>,
) -> axum::response::Response {
    let Json(registration) = match payload {
        Ok(payload) => payload,
        Err(error) => {
            let message = error.body_text();
            return error_response(StatusCode::BAD_REQUEST, "validation", &message);
        }
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.register_device(&registration).await {
        Ok(result) => {
            if result.notify_addition {
                publish_alert(&state, "device-addition", result.device.clone());
            }
            if result.notify_approval {
                publish_alert(&state, "device-approval", result.device.clone());
            }
            (
                StatusCode::CREATED,
                Json(DeviceRegistrationResponse {
                    success: true,
                    data: result.device,
                }),
            )
                .into_response()
        }
        Err(error) => repository_error(error, "Failed to register device"),
    }
}

fn publish_alert(state: &AppState, subtype: &str, device: serde_json::Value) {
    if let Err(error) = state
        .event_publisher
        .publish(ServerEvent::alert(subtype, device))
    {
        tracing::warn!(%error, subtype, "failed to publish device registration alert");
    }
}
