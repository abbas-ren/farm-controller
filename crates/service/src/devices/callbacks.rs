//! Public EdgeController callback HTTP handlers (no bearer auth).
//!
//! Ports Node `DeviceController` routes used by edge hardware at boot and during IPL:
//! `POST /api/v1/device/controller`, `POST /mapping-gen5`, and Gen4/Gen5
//! `flash-confirm` / `flash-confirm-gen4` (GET query and POST body variants) from
//! `farm_controller_old/device-service` `deviceRoutes.ts`.

use std::sync::Arc;

use super::types::TtyEntry;
use super::*;
use crate::error::ErrorResponse;
use crate::socket::helpers::{publish_frontend_event, publish_ipl_progress};
use crate::socket::types::frontend::{AlertLevel, FrontendMessagePayload, TestStatus};
use crate::state::AppState;
use crate::workers::types::DeviceState;
use crate::{devices::types::IplFlashingStatus, socket::types::farm::ServerEvent};
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

/// PURPOSE: Build the shared reqwest client for EdgeController callback HTTP calls.
/// USE: Handlers that POST confirmation/delete/power commands to EdgeController.
pub(super) fn callback_client() -> reqwest::Client {
    let _api_trace = crate::observability::ApiTrace::enter("devices::callbacks", "callback_client");
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
/// PURPOSE: Register or refresh an EdgeController and replace its relay inventory.
/// USE: Called by EdgeController at boot (public, no bearer); publishes frontend alerts and syncs IPL.
pub async fn register_controller(
    State(state): State<Arc<AppState>>,
    Json(registration): Json<ControllerRegistration>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::callbacks", "register_controller");
    let Some(repository) = &state.device_repository else {
        // Guard: device repository is not wired in this process.
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
            // Publish: notify connected frontend clients via Socket.IO.
            if result.created
                && let Err(error) = state.event_publisher.publish(ServerEvent::frontend(
                    FrontendMessagePayload::AlertDataPayload {
                        level: Some(AlertLevel::Info),
                        subtype: Some("device-controller-addition".to_string()),
                        message: serde_json::to_value(&result.controller).unwrap_or_default(),
                        timestamp: Some(chrono::Utc::now().to_rfc3339()),
                    },
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
            let controller_port = state.config.device.edge_controller_http_port;
            if let Ok(url) = reqwest::Url::parse(&format!(
                "http://{}:{controller_port}/confirmation",
                result.controller.ip_address
            )) {
                // Edge: outbound HTTP call to EdgeController.
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
        // Map: translate repository failure into the legacy HTTP error envelope.
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
/// PURPOSE: Persist a Gen5 UART/power mapping result from EdgeController.
/// USE: Public EdgeController callback during Gen5 mapping workflows.
pub async fn mapping_gen5(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(callback): Json<MappingCallback>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::callbacks", "mapping_gen5");
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
    post,
    path = "/api/v1/device/flash-confirm",
    tag = "Devices",
    summary = "Confirm a Gen5 IPL flash",
    description = "Public EdgeController callback that commits the Gen5 IPL flash outcome before downstream worker processing continues.",
    request_body(content = IplFlashingStatus, example = json!({
        "mac": "aabbccddeeff",
        "status": "success",
        "completed": false,
        "message": "Flash in progress",
        "percentage": 50,
        "generation": 5
    })),
    responses(
        (status = 200, description = "Confirmation processed", body = CallbackResponse),
        (status = 500, description = "Confirmation persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
/// PURPOSE: Commit Gen5 IPL flash progress/outcome from EdgeController.
/// USE: Public EdgeController POST callback during Gen5 IPL flashing.
pub async fn flash_confirm(
    State(state): State<Arc<AppState>>,
    Json(flashing_status): Json<IplFlashingStatus>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::callbacks", "flash_confirm");
    flash_confirmation_response(&state, "x5h", flashing_status).await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/flash-confirm",
    tag = "Devices",
    summary = "Confirm a Gen5 IPL flash (legacy query)",
    params(FlashConfirmationQuery),
    responses(
        (status = 200, description = "Confirmation processed", body = CallbackResponse),
        (status = 500, description = "Confirmation persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
/// PURPOSE: Legacy Gen5 IPL flash confirmation via query-string status.
/// USE: Older EdgeController/Node clients that GET /flash-confirm.
pub async fn flash_confirm_get(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FlashConfirmationQuery>,
) -> axum::response::Response {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::callbacks", "flash_confirm_get");
    flash_confirmation_response(&state, "x5h", legacy_flash_status(query.status, 5)).await
}

#[utoipa::path(
    post,
    path = "/api/v1/device/flash-confirm-gen4",
    tag = "Devices",
    summary = "Confirm a Gen4 IPL flash",
    description = "Public EdgeController callback that commits the Gen4 IPL flash outcome before downstream worker processing continues.",
    request_body(content = IplFlashingStatus, example = json!({
        "mac": "aabbccddeeff",
        "status": "success",
        "completed": false,
        "message": "Flash in progress",
        "percentage": 50,
        "generation": 4
    })),
    responses(
        (status = 200, description = "Confirmation processed", body = CallbackResponse),
        (status = 500, description = "Confirmation persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
/// PURPOSE: Commit Gen4 IPL flash progress/outcome from EdgeController.
/// USE: Public EdgeController POST callback during Gen4 IPL flashing.
pub async fn flash_confirm_gen4(
    State(state): State<Arc<AppState>>,
    Json(flashing_status): Json<IplFlashingStatus>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::callbacks", "flash_confirm_gen4");
    tracing::debug!(
        mac = %flashing_status.mac,
        percentage = flashing_status.percentage,
        completed = flashing_status.completed,
        "received Gen4 IPL flash confirmation"
    );
    flash_confirmation_response(&state, "v4h", flashing_status).await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/flash-confirm-gen4",
    tag = "Devices",
    summary = "Confirm a Gen4 IPL flash (legacy query)",
    params(FlashConfirmationQuery),
    responses(
        (status = 200, description = "Confirmation processed", body = CallbackResponse),
        (status = 500, description = "Confirmation persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
/// PURPOSE: Legacy Gen4 IPL flash confirmation via query-string status.
/// USE: Older EdgeController/Node clients that GET /flash-confirm-gen4.
pub async fn flash_confirm_gen4_get(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FlashConfirmationQuery>,
) -> axum::response::Response {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::callbacks", "flash_confirm_gen4_get");
    flash_confirmation_response(&state, "v4h", legacy_flash_status(query.status, 4)).await
}

/// PURPOSE: Synthesize an IplFlashingStatus body from a legacy GET query status.
/// USE: GET flash-confirm handlers before calling flash_confirmation_response.
fn legacy_flash_status(status: CallbackStatus, generation: u8) -> IplFlashingStatus {
    IplFlashingStatus {
        status,
        percentage: 100,
        completed: true,
        mac: String::new(),
        generation,
        message: String::new(),
    }
}

/// PURPOSE: Run a repository callback and return the standard success/error JSON envelope.
/// USE: mapping_gen5 and similar public callbacks.
async fn callback_response<F, Fut>(
    state: &AppState,
    operation: F,
    message: &'static str,
) -> axum::response::Response
where
    F: FnOnce(Arc<dyn DeviceRepository>) -> Fut,
    Fut: std::future::Future<Output = Result<(), DeviceRepositoryError>>,
{
    let _api_trace = crate::observability::ApiTrace::enter("devices::callbacks", "callback_response");
    let Some(repository) = state.device_repository.clone() else {
        // Guard: device repository is not wired in this process.
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let message = message.to_string();
    match operation(repository).await {
        Ok(()) => Json(CallbackResponse {
            success: true,
            message,
        })
        .into_response(),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "Callback failed"),
    }
}

/// PURPOSE: Persist flash confirmation, cancel/update executions, and publish frontend events.
/// USE: All Gen4/Gen5 flash-confirm HTTP entry points.
async fn flash_confirmation_response(
    state: &AppState,
    device_type: &str,
    flashing_status: IplFlashingStatus,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::callbacks", "flash_confirmation_response");
    let Some(repository) = state.device_repository.as_ref() else {
        // Guard: device repository is not wired in this process.
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository
        .confirm_flash(device_type, &flashing_status)
        .await
    {
        Ok(result) => {
            tracing::debug!(
                device_type,
                percentage = flashing_status.percentage,
                completed = flashing_status.completed,
                "IPL flash confirmation applied"
            );
            if let Some(execution) = result.cancelled_execution {
                let test_id = execution.test_id;
                let fail_message =
                    "IPL flashing failed on the device. The test was cancelled — please check the hardware setup and try again.";
                if let Err(error) = crate::devices::test_log::append_test_log(
                    &state,
                    &test_id,
                    fail_message,
                )
                .await
                {
                    tracing::warn!(
                        %error,
                        test_id = %test_id,
                        "failed to append IPL failure test log"
                    );
                }
                publish_frontend_event(
                    state,
                    FrontendMessagePayload::TestExecutionUpdate {
                        build_id: Some(execution.build_id),
                        test_id: test_id.clone(),
                        status: Some(TestStatus::Cancelled),
                        progress: None,
                        message: Some(fail_message.to_owned()),
                        cancel_requested: None,
            progress_kind: None,
            phase: None,
                    },
                );
            }
            if let Some(device_id) = result.freed_device_id {
                publish_frontend_event(
                    state,
                    FrontendMessagePayload::DeviceStateUpdate {
                        device_id: device_id.clone(),
                        state: DeviceState::Free,
                        upgrading: false,
                        flashing: false,
                        changed_at: chrono::Utc::now().to_rfc3339(),
                    },
                );
            }

            if let Some(progress) = result.progress {
                let test_id = progress.test_id;
                let build_id = progress.build_id;
                let percentage = (progress.percentage as u64).min(100);
                let log_message = if progress.message.is_empty() {
                    if progress.completed {
                        "IPL flashing finished successfully. Waiting for the device to come back online before starting the selected tests..."
                            .to_owned()
                    } else {
                        format!("IPL flashing in progress: {percentage}%")
                    }
                } else if progress.completed
                    && !progress.message.to_ascii_lowercase().contains("waiting")
                {
                    format!(
                        "{} Waiting for the device to come back online before starting the selected tests...",
                        progress.message.trim()
                    )
                } else {
                    progress.message.clone()
                };
                if let Err(error) = crate::devices::test_log::append_test_log(
                    &state,
                    &test_id,
                    &log_message,
                )
                .await
                {
                    tracing::warn!(%error, test_id = %test_id, "failed to append IPL progress log");
                }
                // Keep status inProgress at 100% — selected tests still need to run.
                publish_ipl_progress(
                    state,
                    &test_id,
                    Some(&build_id),
                    percentage,
                    &log_message,
                );
            }
            // Wake workers so post-confirm requeue / cancel side effects are claimed promptly.
            state.wake_workers();
            let message = flashing_status.message.clone();
            Json(CallbackResponse {
                success: true,
                message,
            })
            .into_response()
        }
        Err(error) => {
            tracing::error!(%error, "Callback failed");
            // Map: translate repository failure into the legacy HTTP error envelope.
            repository_error(error, "Callback failed")
        }
    }
}

/// PURPOSE: Parse optional tty_entry JSON (object or stringified) into TtyEntry.
/// USE: mapping_gen5 request validation.
fn parse_tty_entry(value: Option<&serde_json::Value>) -> Result<Option<TtyEntry>, String> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::callbacks", "parse_tty_entry");
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
