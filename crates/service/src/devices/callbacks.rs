use std::sync::Arc;

use super::types::TtyEntry;
use super::*;
use crate::error::ErrorResponse;
use crate::{events::ServerEvent, state::AppState};
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

pub(super) fn callback_client() -> reqwest::Client {
    crate::external_http::client(DEVICE_CALLBACK_TIMEOUT)
        .expect("static device callback HTTP policy is valid")
}

#[utoipa::path(
    post,
    path = "/api/v1/device/controller/",
    tag = "Devices",
    summary = "Register an EdgeController",
    description = "Creates or refreshes an EdgeController and atomically replaces its relay inventory. This compatibility route is intentionally public because deployed EdgeControllers do not send bearer tokens.",
    request_body(content = ControllerRegistration, example = json!({
        "macAddress": "aa:bb:cc:dd:ee:ff",
        "ipAddress": "192.0.2.10",
        "deviceFamily": "Gen5",
        "relays": [{"serialNumber": "A100", "state": {"channel_1": 0}}],
        "uid": "aabbccddeeff"
    })),
    responses(
        (status = 201, description = "Controller created", body = ControllerRegistrationResponse),
        (status = 200, description = "Existing controller refreshed", body = ControllerRegistrationResponse),
        (status = 400, description = "Invalid registration", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn register_controller(
    State(state): State<Arc<AppState>>,
    Json(registration): Json<ControllerRegistration>,
) -> axum::response::Response {
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.register_controller(&registration).await {
        Ok(result) => {
            let action = if result.created { "added" } else { "updated" };
            publish_controller_changed(&state, action, &result.controller.device_controller_id);
            if result.created
                && let Err(error) = state.event_publisher.publish(ServerEvent::alert(
                    "device-controller-addition",
                    serde_json::to_value(&result.controller).unwrap_or_default(),
                ))
            {
                tracing::warn!(%error, controller_id = %result.controller.device_controller_id, "failed to publish new-controller alert");
            }
            if let Some(device_family) = result.controller.device_family.as_deref()
                && let Err(error) = crate::workers::sync_controller_ipl(
                    &state,
                    &result.controller.ip_address,
                    device_family,
                )
                .await
            {
                tracing::error!(%error, controller_id = %result.controller.device_controller_id, "failed to synchronize IPL payloads to registered controller");
            }
            if let Ok(url) = reqwest::Url::parse(&format!(
                "http://{}:{EDGE_CONTROLLER_PORT}/confirmation",
                result.controller.ip_address
            )) {
                match callback_client()
                    .post(url)
                    .json(&serde_json::json!({
                        "controllerId": result.controller.device_controller_id,
                    }))
                    .send()
                    .await
                {
                    Ok(response) if response.status().is_success() => {}
                    Ok(response) => tracing::warn!(
                        status = %response.status(),
                        controller_id = %result.controller.device_controller_id,
                        "controller registration confirmation was rejected"
                    ),
                    Err(error) => tracing::warn!(
                        %error,
                        controller_id = %result.controller.device_controller_id,
                        "controller saved but registration confirmation failed"
                    ),
                }
            }
            (
                if result.created {
                    StatusCode::CREATED
                } else {
                    StatusCode::OK
                },
                Json(ControllerRegistrationResponse {
                    success: true,
                    data: result.controller,
                }),
            )
                .into_response()
        }
        Err(error) => repository_error(error, "Failed to register device controller"),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/device/mapping-gen5",
    tag = "Devices",
    summary = "Receive a Gen5 mapping result",
    description = "Public EdgeController callback. Persists the Gen5 UART/power mapping result; the caller IP is taken from the payload or first forwarded address.",
    request_body(content = MappingCallback, example = json!({
        "mac": "aabbccddeeff",
        "status": "success",
        "tty_entry": {"uart": "/dev/ttyUSB0", "power": "1"},
        "ip": "192.0.2.10"
    })),
    responses(
        (status = 200, description = "Mapping callback processed", body = CallbackResponse),
        (status = 400, description = "Invalid callback", body = ErrorResponse),
        (status = 500, description = "Mapping persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn mapping_gen5(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(callback): Json<MappingCallback>,
) -> axum::response::Response {
    let caller_ip = callback.ip.clone().or_else(|| {
        headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .map(str::to_owned)
    });
    let tty_entry = match parse_tty_entry(callback.tty_entry.as_ref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, "validation", &message),
    };
    callback_response(
        &state,
        |repository| async move {
            repository
                .save_gen5_mapping(
                    caller_ip.as_deref().unwrap_or_default(),
                    &callback.mac,
                    callback.status,
                    tty_entry.as_ref(),
                )
                .await
        },
        "Gen5 mapping processed",
    )
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/flash-confirm",
    tag = "Devices",
    summary = "Confirm a Gen5 IPL flash",
    description = "Public EdgeController callback that commits the Gen5 IPL flash outcome before downstream worker processing continues.",
    params(("status" = CallbackStatus, Query, description = "Flash result")),
    responses(
        (status = 200, description = "Confirmation processed", body = CallbackResponse),
        (status = 500, description = "Confirmation persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn flash_confirm(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FlashConfirmationQuery>,
) -> axum::response::Response {
    flash_confirmation_response(
        &state,
        "x5h",
        query.status,
        "Flash confirmation handled successfully",
    )
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/flash-confirm-gen4",
    tag = "Devices",
    summary = "Confirm a Gen4 IPL flash",
    description = "Public EdgeController callback that commits the Gen4 IPL flash outcome before downstream worker processing continues.",
    params(("status" = CallbackStatus, Query, description = "Flash result")),
    responses(
        (status = 200, description = "Confirmation processed", body = CallbackResponse),
        (status = 500, description = "Confirmation persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn flash_confirm_gen4(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FlashConfirmationQuery>,
) -> axum::response::Response {
    flash_confirmation_response(
        &state,
        "v4h",
        query.status,
        "Flash confirmation for Gen4 handled successfully",
    )
    .await
}

async fn callback_response<F, Fut>(
    state: &AppState,
    operation: F,
    message: &'static str,
) -> axum::response::Response
where
    F: FnOnce(Arc<dyn DeviceRepository>) -> Fut,
    Fut: std::future::Future<Output = Result<(), DeviceRepositoryError>>,
{
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match operation(repository).await {
        Ok(()) => Json(CallbackResponse {
            success: true,
            message,
        })
        .into_response(),
        Err(error) => repository_error(error, "Callback failed"),
    }
}

async fn flash_confirmation_response(
    state: &AppState,
    device_type: &str,
    status: CallbackStatus,
    message: &'static str,
) -> axum::response::Response {
    let Some(repository) = state.device_repository.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.confirm_flash(device_type, status).await {
        Ok(result) => {
            if let Some(execution) = result.cancelled_execution {
                let test_id = execution.test_id;
                let created_by = execution.created_by;
                if let Err(error) = state.event_publisher.publish(ServerEvent {
                    event: "test_execution_update".to_owned(),
                    payload: serde_json::json!({
                        "testId": test_id,
                        "buildId": execution.build_id,
                        "update": {
                            "testId": test_id,
                            "type": "status",
                            "data": {
                                "status": "cancelled",
                                "timestamp": chrono::Utc::now().to_rfc3339(),
                            }
                        },
                        "createdBy": created_by,
                    }),
                    room: Some(format!("user:{created_by}")),
                }) {
                    tracing::warn!(%error, test_id, "failed to publish IPL rejection cancellation");
                }
            }
            if let Some(device_id) = result.freed_device_id
                && let Err(error) = state.event_publisher.publish(ServerEvent {
                    event: "device_state_update".to_owned(),
                    payload: serde_json::json!({
                        "deviceId": device_id,
                        "state": "free",
                        "upgrading": false,
                        "flashing": false,
                        "changedAt": chrono::Utc::now().to_rfc3339(),
                    }),
                    room: None,
                })
            {
                tracing::warn!(%error, device_id, "failed to publish IPL rejection device state");
            }
            Json(CallbackResponse {
                success: true,
                message,
            })
            .into_response()
        }
        Err(error) => repository_error(error, "Callback failed"),
    }
}

fn parse_tty_entry(value: Option<&serde_json::Value>) -> Result<Option<TtyEntry>, String> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => serde_json::from_str(value)
            .map(Some)
            .map_err(|_| "tty_entry must contain a valid JSON mapping".to_owned()),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| "tty_entry must contain uart and power strings".to_owned()),
    }
}
