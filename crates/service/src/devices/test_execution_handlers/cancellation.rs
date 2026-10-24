//! Cancellation command, deferred completion, and cancellation event construction.
//!
//! Ports Node `PUT /api/v1/device/test/cancel/{id}` (`TestController.cancelExecution`).

use super::*;
use crate::{
    auth,
    devices::{
        TEST_CANCELLATION_TIMEOUT, callbacks::callback_client, power_handlers, DeviceRepository,
        repository_types::DevicePowerTarget,
    },
    error::ErrorResponse,
    socket::{
        helpers::device_state_event,
        types::{
            farm::ServerEvent,
            frontend::{
                FrontendMessagePayload, TestExecutionUpdateData, TestExecutionUpdateType, TestStatus,
            },
        },
    },
    state::AppState,
    workers::types::DeviceState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use std::sync::Arc;
use std::time::Duration;

/// PURPOSE: Request test execution cancellation
/// USE: Authenticated frontend (bearer); publishes Socket.IO frontend events.
#[utoipa::path(put, path = "/api/v1/device/test/cancel/{id}", tag = "Tests", summary = "Request test execution cancellation", description = "Persists an idempotent cancellation request before publishing owner and build-room events.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Cancellation requested", body = MessageResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Cancellation failed or execution not found", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn cancel(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::test_execution_handlers::cancellation", "cancel");
    // Auth: require any authenticated bearer token.
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let user_id = auth::token_subject(&headers).unwrap_or_else(|| "system".to_owned());
    let Some(repository) = &state.device_repository else {
        // Guard: device repository is not wired in this process.
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository
        .request_test_cancellation(&test_id, &user_id, &state.config.device.nfs_host_path)
        .await
    {
        Ok(Some(result)) => {
            if result.changed {
                for event in cancellation_events(
                    &test_id,
                    &result.build_id,
                    &result.phase,
                    &result.created_by,
                ) {
                    // Publish: notify connected frontend clients via Socket.IO.
                    if let Err(error) = state.event_publisher.publish(event) {
                        tracing::warn!(%error, test_id, "failed to publish test cancellation");
                    }
                }
            }
            // Always clear NFS currentTest so eagent resume cannot restart a cancelled run.
            if result.changed {
                clear_nfs_current_test(
                    &state.config.device.nfs_host_path,
                    result.device_type.as_deref(),
                    result.device_nfs_path.as_deref(),
                    &test_id,
                )
                .await;
            }
            // Notify eagent when the DUT may still be running/booting for this test.
            if let Some(device_ip) = result.device_ip.as_deref() {
                send_device_cancellation(
                    device_ip,
                    &test_id,
                    state.config.device.edge_agent_http_port,
                )
                .await;
            }
            if result.terminalized {
                if let Some(device_type) = result.cleanup_device_type.as_deref() {
                    cleanup_cancelled_preparation(&state, device_type, &test_id).await;
                    if let Some(run_id) = result
                        .test_cycle_id
                        .as_deref()
                        .and_then(|value| value.parse::<u64>().ok())
                        && let Some(catalog) = state.test_catalog.as_ref()
                        && let Err(error) = catalog.delete_run(run_id).await
                    {
                        tracing::warn!(%error, run_id, test_id, "failed to delete cancelled TestRail run");
                    }
                }
                // Publish: notify connected frontend clients via Socket.IO.
                if let Err(error) = state.event_publisher.publish(ServerEvent::frontend(
                    FrontendMessagePayload::TestExecutionUpdate {
                        build_id: None,
                        test_id: test_id.to_owned(),
                        status: Some(TestStatus::Cancelled),
                        progress: None,
                        message: None,
                        cancel_requested: None,
                        progress_kind: None,
                        phase: None,
                    },
                )) {
                    tracing::warn!(%error, test_id, "failed to publish terminal cancellation");
                }
                if let Some(device_id) = result.device_id.as_deref() {
                    if let Err(error) = state
                        .event_publisher
                        .publish(device_state_event(device_id, DeviceState::Free))
                    {
                        tracing::warn!(%error, device_id, test_id, "failed to publish device free after cancel");
                    }
                    // Best-effort power-cycle so the DUT leaves any stuck busy/boot state.
                    power_cycle_cancelled_device(&state, repository.as_ref(), device_id, &test_id)
                        .await;
                }
            }
            (
                response_headers,
                Json(serde_json::json!({"message": "  Test execution cancelled successfully"})),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Could not cancel test execution",
        ),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "Error while cancelling test execution"),
    }
}

/// PURPOSE: Power-cycle a cancelled DUT via EdgeController so it recovers for reassignment.
/// USE: cancel handler after the device row is marked Free.
async fn power_cycle_cancelled_device(
    state: &AppState,
    repository: &dyn DeviceRepository,
    device_id: &str,
    test_id: &str,
) {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::test_execution_handlers::cancellation",
        "power_cycle_cancelled_device",
    );
    let target = match repository.device_power_target(device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            tracing::warn!(device_id, test_id, "no device found for cancel power-cycle");
            return;
        }
        Err(error) => {
            tracing::warn!(%error, device_id, test_id, "failed to resolve cancel power-cycle target");
            return;
        }
    };
    let Some(controller_ip) = target.controller_ip.as_deref() else {
        tracing::warn!(device_id, test_id, "no controller mapping for cancel power-cycle");
        return;
    };
    let controller_port = state.config.device.edge_controller_http_port;
    let family = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .replace(' ', "")
        .to_lowercase();
    let client = callback_client();
    if family == "gen5" || target.power_port.is_some() {
        let Some(power_port) = target.power_port.as_deref() else {
            tracing::warn!(device_id, test_id, "Gen5 cancel power-cycle missing power port");
            return;
        };
        // EdgeController `/reboot-device` performs POWER#OFF → sleep → POWER#ON.
        let url = match reqwest::Url::parse(&format!(
            "http://{controller_ip}:{controller_port}/reboot-device"
        )) {
            Ok(url) => url,
            Err(_) => {
                tracing::warn!(controller_ip, device_id, test_id, "invalid controller address");
                return;
            }
        };
        match client
            .post(url)
            .json(&serde_json::json!({ "power": power_port }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let _ = repository
                    .apply_device_power(device_id, target.channel_id, "on")
                    .await;
                if let Err(error) = state.event_publisher.publish(power_handlers::device_power_event(
                    device_id, "on",
                )) {
                    tracing::warn!(%error, device_id, "failed to publish power-on after cancel cycle");
                }
                tracing::info!(device_id, test_id, "cancel power-cycle via /reboot-device succeeded");
            }
            Ok(response) => {
                tracing::warn!(
                    status = %response.status(),
                    device_id,
                    test_id,
                    "controller rejected cancel /reboot-device"
                );
            }
            Err(error) => {
                tracing::warn!(%error, device_id, test_id, "cancel /reboot-device request failed");
            }
        }
        return;
    }
    power_cycle_via_relay(state, repository, device_id, test_id, &target, controller_ip).await;
}

/// PURPOSE: Relay off→on power cycle for Gen3/Gen4 cancel recovery.
/// USE: `power_cycle_cancelled_device` when no Gen5 power TTY mapping exists.
async fn power_cycle_via_relay(
    state: &AppState,
    repository: &dyn DeviceRepository,
    device_id: &str,
    test_id: &str,
    target: &DevicePowerTarget,
    controller_ip: &str,
) {
    let (Some(serial), Some(channel)) = (target.relay_serial.as_deref(), target.channel_number)
    else {
        tracing::warn!(device_id, test_id, "no relay mapping for cancel power-cycle");
        return;
    };
    let controller_port = state.config.device.edge_controller_http_port;
    let url = match reqwest::Url::parse(&format!(
        "http://{controller_ip}:{controller_port}/relay"
    )) {
        Ok(url) => url,
        Err(_) => {
            tracing::warn!(controller_ip, device_id, test_id, "invalid controller address");
            return;
        }
    };
    let client = callback_client();
    for state_name in ["off", "on"] {
        match client
            .post(url.clone())
            .json(&serde_json::json!({
                "serial": serial,
                "state": state_name,
                "channel": channel,
            }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let _ = repository
                    .apply_device_power(device_id, target.channel_id, state_name)
                    .await;
                if let Err(error) = state
                    .event_publisher
                    .publish(power_handlers::device_power_event(device_id, state_name))
                {
                    tracing::warn!(%error, device_id, state_name, "failed to publish cancel power event");
                }
            }
            Ok(response) => {
                tracing::warn!(
                    status = %response.status(),
                    device_id,
                    test_id,
                    state_name,
                    "controller rejected cancel relay power command"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    device_id,
                    test_id,
                    state_name,
                    "cancel relay power command failed"
                );
                return;
            }
        }
        if state_name == "off" {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    tracing::info!(device_id, test_id, "cancel relay power-cycle succeeded");
}

/// PURPOSE: Ask EdgeController/EdgeAgent to abort the running test.
/// USE: cancel handler after marking cancellation requested.
async fn send_device_cancellation(device_ip: &str, test_id: &str, agent_port: u16) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::test_execution_handlers::cancellation", "send_device_cancellation");
    let Ok(url) = reqwest::Url::parse(&format!("http://{device_ip}:{agent_port}/cancel"))
    else {
        tracing::warn!(
            device_ip,
            test_id,
            "invalid device address for cancellation"
        );
        return;
    };
    let client = match crate::external_http::client(TEST_CANCELLATION_TIMEOUT) {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, test_id, "failed to create cancellation client");
            return;
        }
    };
    let request = crate::external_http::with_edge_agent_auth(
        client.post(url).json(&serde_json::json!({"testId": test_id})),
    );
    if let Err(error) = request.send().await {
        tracing::warn!(%error, test_id, "device cancellation request failed");
    }
}

/// Remove farm-staged / Mode A `usr/src/currentTest.txt` so post-reboot eagent resume stops.
async fn clear_nfs_current_test(
    nfs_host_path: &str,
    device_type: Option<&str>,
    device_nfs_path: Option<&str>,
    test_id: &str,
) {
    let mut paths = Vec::new();
    if let Some(device_type) = device_type.filter(|value| {
        super::validation::safe_segment(value) && super::validation::safe_segment(test_id)
    }) {
        paths.push(
            std::path::Path::new(nfs_host_path)
                .join(device_type)
                .join(test_id)
                .join("usr/src/currentTest.txt"),
        );
    }
    if let Some(nfs_path) = device_nfs_path.map(str::trim).filter(|value| !value.is_empty()) {
        paths.push(std::path::Path::new(nfs_path).join("usr/src/currentTest.txt"));
    }
    for path in paths {
        if let Err(error) = tokio::fs::remove_file(&path).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, path = %path.display(), test_id, "failed to clear NFS currentTest.txt");
        }
    }
}

/// PURPOSE: Clear preparation state after a cancelled execution.
/// USE: Cancellation completion paths.
async fn cleanup_cancelled_preparation(state: &AppState, device_type: &str, test_id: &str) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::test_execution_handlers::cancellation", "cleanup_cancelled_preparation");
    if !super::validation::safe_segment(device_type) || !super::validation::safe_segment(test_id) {
        tracing::warn!(
            device_type,
            test_id,
            "refusing unsafe cancelled-test cleanup path"
        );
        return;
    }
    let path = std::path::Path::new(&state.config.device.nfs_host_path)
        .join(device_type)
        .join(test_id);
    if let Err(error) = tokio::fs::remove_dir_all(&path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %path.display(), test_id, "failed to remove cancelled test artifacts");
    }
}

/// PURPOSE: Finish a cancellation that waited for device acknowledgement.
/// USE: Workers/handlers when cancel ack arrives.
pub(crate) async fn complete_deferred_cancellation(
    state: &AppState,
    cancellation: &super::super::repository_types::DeferredCancellation,
) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::test_execution_handlers::cancellation", "complete_deferred_cancellation");
    // Publish: notify connected frontend clients via Socket.IO.
    if let Err(error) = state.event_publisher.publish(ServerEvent::frontend(
        FrontendMessagePayload::TestExecutionUpdate {
            build_id: Some(cancellation.build_id.clone()),
            test_id: cancellation.test_id.clone(),
            status: Some(TestStatus::Cancelled),
            progress: None,
            message: None,
            cancel_requested: None,
            progress_kind: None,
            phase: None,
        },
    )) {
        tracing::warn!(
            %error,
            test_id = cancellation.test_id,
            "failed to publish deferred terminal cancellation"
        );
    }
}

/// PURPOSE: Build Socket.IO payloads for cancellation progress/completion.
/// USE: cancel and deferred-completion paths.
pub(crate) fn cancellation_events(
    test_id: &str,
    build_id: &str,
    phase: &str,
    _user_id: &str,
) -> [ServerEvent; 2] {
    [
        ServerEvent::frontend(FrontendMessagePayload::TestExecution {
            test_id: test_id.to_owned(),
            r#type: TestExecutionUpdateType::CancelRequested,
            data: TestExecutionUpdateData {
                status: Some(TestStatus::CancelRequested),
                timestamp: chrono::Utc::now().to_rfc3339(),
                cancel_requested: Some(true),
                phase: Some(phase.to_owned()),
                rtos_log_path: None,
                message: None,
            },
        }),
        ServerEvent::frontend(FrontendMessagePayload::TestExecutionUpdate {
            build_id: Some(build_id.to_owned()),
            test_id: test_id.to_owned(),
            status: Some(TestStatus::CancelRequested),
            progress: None,
            message: None,
            cancel_requested: Some(true),
            progress_kind: None,
            phase: None,
        }),
    ]
}
