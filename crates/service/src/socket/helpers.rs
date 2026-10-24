//! Event builders and broadcast helpers used by workers and HTTP handlers.
//!
//! Shapes assignment, failure, device/owner/build refresh events for the UI.

use axum::http::{HeaderMap, HeaderValue, header};
use socketioxide::{
    SocketIo,
    extract::{SocketRef, TryData},
    layer::SocketIoLayer,
};
use uuid::Uuid;

use crate::{
    devices::{ControllerHeartbeat, DeviceHeartbeatResult},
    socket::{
        constants::BROWSER_BEARER_PREFIX,
        types::{
            farm::{BrowserAuthorization, EventHub, NativeSocketQuery, ServerEvent},
            frontend::{
                Action, AlertLevel, CpuMetrics, DiskMetrics, FrontendMessagePayload, HeartbeatData,
                InterfaceChange, MemoryMetrics, Metrics, NetworkMetrics, TestExecutionUpdateData,
                TestExecutionUpdateType, TestStatus,
            },
        },
    },
    state::AppState,
    workers::types::{
        AssignedAction, ControllerStateTransition, DeviceState, DeviceStateTransition,
        ExpiredRelayConfiguration,
    },
};

/// Browser authorization.
pub(crate) fn browser_authorization(query: &NativeSocketQuery) -> Result<BrowserAuthorization, ()> {
    match (
        query.client.as_deref(),
        query.admin_terminal.unwrap_or(false),
    ) {
        (Some("frontend"), true) => Ok(BrowserAuthorization::Admin),
        (Some("frontend" | "cli"), false) => Ok(BrowserAuthorization::Authenticated),
        (_, true) => Err(()),
        _ => Ok(BrowserAuthorization::None),
    }
}

/// Browser auth headers.
pub(crate) fn browser_auth_headers(headers: &HeaderMap) -> HeaderMap {
    let mut authorized = headers.clone();
    if authorized.contains_key(header::AUTHORIZATION) {
        return authorized;
    }
    let token = headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .and_then(|protocols| {
            protocols
                .split(',')
                .map(str::trim)
                .find_map(|protocol| protocol.strip_prefix(BROWSER_BEARER_PREFIX))
        })
        .filter(|token| !token.is_empty());
    if let Some(token) = token
        && let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}"))
    {
        authorized.insert(header::AUTHORIZATION, value);
    }
    authorized
}

/// Socket io layer.
pub(crate) fn socket_io_layer(event_hub: &EventHub) -> SocketIoLayer {
    let (layer, socket_io) = SocketIo::builder()
        .max_payload(1024 * 1024)
        .max_buffer_size(256)
        .build_layer();
    socket_io.ns("/", socket_io_connect);
    event_hub.install_socket_io(socket_io);
    layer
}

/// Socket io connect.
async fn socket_io_connect(socket: SocketRef, TryData(auth): TryData<serde_json::Value>) {
    if let Ok(_auth) = auth {
        // Validate/authenticate `auth` here if required.
        tracing::debug!(socket_id = %socket.id, "Socket.IO client connected");
    } else {
        tracing::warn!(
            socket_id = %socket.id,
            "Socket.IO connection received invalid auth data"
        );
    }
}

/// Normalize controller id.
pub(crate) fn normalize_controller_id(value: &str) -> String {
    value.trim().replace([':', '-'], "").to_lowercase()
}

/// Publish frontend event through the centralized EventHub (minimal tracing).
pub(crate) fn publish_frontend_event(state: &AppState, payload: FrontendMessagePayload) {
    if let Err(error) = state
        .event_publisher
        .publish(ServerEvent::frontend(payload))
    {
        tracing::warn!(%error, "failed to publish frontend event");
    }
}

/// Publish IPL flash progress (`testExecutionUpdate.progress`) for the frontend bar.
///
/// Status stays `inProgress` even at 100% — IPL completion is not test completion.
pub(crate) fn publish_ipl_progress(
    state: &AppState,
    test_id: &str,
    build_id: Option<&str>,
    percentage: u64,
    message: &str,
) {
    publish_frontend_event(
        state,
        FrontendMessagePayload::TestExecutionUpdate {
            build_id: build_id.map(str::to_owned),
            test_id: test_id.to_owned(),
            status: Some(TestStatus::InProgress),
            progress: Some(percentage.min(100)),
            message: Some(message.to_owned()),
            cancel_requested: None,
        },
    );
}

/// Publish live test-case / overall result progress for the Results UI.
pub(crate) fn publish_result_progress(
    state: &AppState,
    test_id: &str,
    build_id: Option<&str>,
    percentage: u64,
    status: TestStatus,
    message: Option<&str>,
) {
    publish_frontend_event(
        state,
        FrontendMessagePayload::TestExecutionUpdate {
            build_id: build_id.map(str::to_owned),
            test_id: test_id.to_owned(),
            status: Some(status),
            progress: Some(percentage.min(100)),
            message: message.map(str::to_owned),
            cancel_requested: None,
        },
    );
}

/// Map raw device/dispatch errors into short user-facing test-log lines.
pub(crate) fn user_friendly_dispatch_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("relay") || lower.contains("gpio") {
        "This device is missing relay or GPIO configuration. Ask an administrator to finish hardware setup, then retry."
            .to_owned()
    } else if lower.contains("400")
        && (lower.contains("image") || lower.contains("dtb") || lower.contains("path"))
    {
        "The device rejected the test start request because the flash paths were invalid. Please retry once the device is online."
            .to_owned()
    } else if lower.contains("400") {
        "The device could not start the selected tests (bad request). Check device logs and try again."
            .to_owned()
    } else if lower.contains("401") || lower.contains("unauthorized") {
        "The device rejected the farm request (unauthorized). Check edge-agent security settings."
            .to_owned()
    } else if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection")
        || lower.contains("connect")
    {
        "Could not reach the device. It may still be rebooting after IPL — waiting for it to come back online."
            .to_owned()
    } else if lower.contains("build version is missing") {
        "Cannot start tests because the build version is missing on this execution."
            .to_owned()
    } else {
        format!("Could not continue the test on the device: {error}")
    }
}

// ---------------- Socket Events ------------------

/// Build performance event.
pub(crate) fn build_performance_event(build_id: &str) -> ServerEvent {
    ServerEvent::frontend(FrontendMessagePayload::BuildPerformanceUpdate {
        build_id: build_id.to_owned(),
    })
}

/// Controller heartbeat payload.
pub(crate) fn controller_heartbeat_payload(
    controller_id: String,
    heartbeat: &ControllerHeartbeat,
) -> FrontendMessagePayload {
    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp(heartbeat.timestamp, 0)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| heartbeat.timestamp.to_string());
    FrontendMessagePayload::DeviceControllerPing {
        controller_id,
        timestamp: timestamp.clone(),
        metrics: Metrics {
            cpu: CpuMetrics {
                current: heartbeat.cpu_current.clone(),
                total: heartbeat.cpu_total.clone(),
                usage_percent: heartbeat.cpu_usage_percent,
            },
            memory: MemoryMetrics {
                used: heartbeat.memory_used.clone(),
                total: heartbeat.memory_total.clone(),
                usage_percent: heartbeat.memory_usage_percent,
            },
            network: NetworkMetrics {
                upload: heartbeat.network_upload.clone(),
                download: heartbeat.network_download.clone(),
            },
            disk: DiskMetrics {
                used: heartbeat.disk_used.clone(),
                total: heartbeat.disk_total.clone(),
                usage_percent: heartbeat.disk_usage_percent,
            },
            timestamp,
        },
    }
}

/// Device heartbeat events.
pub(crate) fn device_heartbeat_events(
    device_id: &str,
    result: DeviceHeartbeatResult,
    timestamp: &str,
) -> Vec<ServerEvent> {
    let mut events = Vec::new();

    if result.state_changed {
        events.push(ServerEvent::frontend(
            FrontendMessagePayload::DeviceStateUpdate {
                device_id: device_id.to_owned(),
                state: DeviceState::from(&result.state),
                upgrading: false,
                flashing: false,
                changed_at: timestamp.to_owned(),
            },
        ));
    }

    if !result.interface_changes.is_empty() {
        events.push(ServerEvent::frontend(
            FrontendMessagePayload::DeviceInterfaceUpdate {
                device_id: device_id.to_owned(),
                changes: result
                    .interface_changes
                    .into_iter()
                    .map(|change| {
                        serde_json::from_value::<InterfaceChange>(change).unwrap_or(
                            InterfaceChange {
                                r#type: "".to_owned(),
                                interface_id: None,
                                status: "".to_owned(),
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
                heartbeat_data: serde_json::from_value::<HeartbeatData>(result.heartbeat_data)
                    .unwrap_or_default(),
                timestamp: timestamp.to_owned(),
            },
        ));
    }

    if !result.post_update_events.is_empty() {
        events.extend(result.post_update_events.into_iter());
    }

    events.extend(result.alerts.into_iter().map(|alert| {
        ServerEvent::frontend(FrontendMessagePayload::AlertDataPayload {
            level: Some(AlertLevel::Info),
            subtype: None,
            message: alert,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        })
    }));

    events.push(ServerEvent::frontend(
        FrontendMessagePayload::HeartbeatData {
            device_id: Some(device_id.to_owned()),
            controller_id: None,
            device_controller_id: None,
            timestamp: Some(timestamp.to_owned()),
            status: None,
            extra: None,
        },
    ));

    // Migrate post_update_events at their source to GeneralPayload variants.
    // Do not forward their old arbitrary event names into the new typed protocol.

    events
}

// ---------------- build_preparation ----------------

/// Unit test: test failure events.
pub(crate) fn test_failure_events(
    test_id: &str,
    build_id: &str,
    _user_id: &str,
) -> Vec<ServerEvent> {
    let mut events = vec![ServerEvent::frontend(
        FrontendMessagePayload::TestExecution {
            test_id: test_id.to_owned(),
            r#type: TestExecutionUpdateType::Status,
            data: TestExecutionUpdateData {
                status: Some(TestStatus::Failed),
                message: Some("Test failed".to_owned()),
                timestamp: chrono::Utc::now().to_rfc3339(),
                cancel_requested: Some(false),
                phase: Some("execution".to_owned()),
                rtos_log_path: None,
            },
        },
    )];
    events.push(ServerEvent::frontend(
        FrontendMessagePayload::TestExecutionUpdate {
            test_id: test_id.to_owned(),
            build_id: Some(build_id.to_owned()),
            status: Some(TestStatus::Failed),
            progress: None,
            message: None,
            cancel_requested: Some(false),
        },
    ));
    events.push(build_performance_event(build_id));

    events
}

/// Build failure event.
pub(crate) fn build_failure_event(
    release_id: uuid::Uuid,
    device_id: Option<&str>,
    device_type: &str,
    version: Option<&str>,
    user_id: &str,
) -> ServerEvent {
    ServerEvent::frontend(FrontendMessagePayload::BuildStatusChanged {
        release_id,
        status: "failed".to_owned(),
        is_faulty: true,
        device_id: device_id.map(|s| s.to_owned()),
        device_type: device_type.to_owned(),
        version: version.map(|s| s.to_owned()),
        user_id: user_id.to_owned(),
    })
}

// ------------- device_action ---------------

/// Assignment events.
pub(crate) fn assignment_events(assigned: &AssignedAction) -> Vec<ServerEvent> {
    let mut events = vec![device_state_event(&assigned.device_id, DeviceState::Busy)];
    if let Some(test_id) = assigned.action.test_id.as_deref() {
        let build_id = assigned.action.build_id.as_deref().unwrap_or_default();
        let owner = assigned
            .action
            .created_by
            .as_deref()
            .unwrap_or(&assigned.action.user_id);
        events.push(test_status_event(test_id, TestStatus::InProgress, owner));
        events.push(test_build_event(test_id, build_id, "in_progress"));
        events.push(build_performance_event(build_id));
    } else if let Some(release_id) = assigned.action.release_id {
        events.push(build_status_event(assigned, release_id, "testing", false));
    }
    events
}

/// Dispatch failure events.
pub(crate) fn dispatch_failure_events(assigned: &AssignedAction, error: &str) -> Vec<ServerEvent> {
    let mut events = vec![device_state_event(&assigned.device_id, DeviceState::Free)];
    if let Some(test_id) = assigned.action.test_id.as_deref() {
        let build_id = assigned.action.build_id.as_deref().unwrap_or_default();
        let owner = assigned
            .action
            .created_by
            .as_deref()
            .unwrap_or(&assigned.action.user_id);
        let mut status = test_status_event(test_id, TestStatus::Failed, owner);
        status.payload["data"]["message"] = serde_json::Value::String(error.to_owned());
        events.push(status);
        events.push(test_build_event(test_id, build_id, "failed"));
        events.push(build_performance_event(build_id));
    } else if let Some(release_id) = assigned.action.release_id {
        events.push(build_status_event(assigned, release_id, "failed", true));
    }
    events
}

/// Device state event.
pub(crate) fn device_state_event(device_id: &str, state: DeviceState) -> ServerEvent {
    ServerEvent::frontend(FrontendMessagePayload::DeviceStateUpdate {
        device_id: device_id.to_owned(),
        state,
        upgrading: false,
        flashing: false,
        changed_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Unit test: test status event.
pub(crate) fn test_status_event(test_id: &str, status: TestStatus, _user_id: &str) -> ServerEvent {
    ServerEvent::frontend(FrontendMessagePayload::TestExecution {
        test_id: test_id.to_owned(),
        r#type: TestExecutionUpdateType::Status,
        data: TestExecutionUpdateData {
            status: Some(status),
            timestamp: chrono::Utc::now().to_rfc3339(),
            cancel_requested: None,
            phase: None,
            rtos_log_path: None,
            message: None,
        },
    })
}

/// Unit test: test build event.
pub(crate) fn test_build_event(test_id: &str, build_id: &str, status: &str) -> ServerEvent {
    ServerEvent::frontend(FrontendMessagePayload::TestExecutionUpdate {
        build_id: Some(build_id.to_owned()),
        test_id: test_id.to_owned(),
        status: Some(TestStatus::from(status)),
        progress: None,
        message: None,
        cancel_requested: None,
    })
}

/// Build status event.
pub(crate) fn build_status_event(
    assigned: &AssignedAction,
    release_id: Uuid,
    status: &str,
    is_faulty: bool,
) -> ServerEvent {
    ServerEvent::frontend(FrontendMessagePayload::BuildStatusChanged {
        release_id,
        status: status.to_owned(),
        is_faulty: is_faulty,
        device_id: Some(assigned.device_id.to_owned()),
        device_type: assigned.action.device_type.to_owned(),
        version: assigned.action.version.clone(),
        user_id: assigned.action.user_id.to_owned(),
    })
}

// ---------------- device_heartbeat ------------------

/// Transition events.
pub(crate) fn transition_events(transition: &DeviceStateTransition) -> [ServerEvent; 2] {
    [
        ServerEvent::frontend(FrontendMessagePayload::DeviceStateUpdate {
            device_id: transition.device_id.clone(),
            state: transition.state.clone(),
            upgrading: false,
            flashing: false,
            changed_at: chrono::Utc::now().to_rfc3339(),
        }),
        ServerEvent::frontend(FrontendMessagePayload::AlertDataPayload {
            level: Some(AlertLevel::Warning),
            subtype: None,
            message: serde_json::json!({"alert": "device alert"}),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        }),
    ]
}

// ---------------- device_controller ------------------

/// Controller timeout events.
pub(crate) fn controller_timeout_events(
    transition: &ControllerStateTransition,
) -> [ServerEvent; 2] {
    [
        ServerEvent::frontend(FrontendMessagePayload::DeviceControllerChanged {
            action: Action::Updated,
            controller_id: transition.controller_id.clone(),
        }),
        ServerEvent::frontend(FrontendMessagePayload::AlertDataPayload {
            level: Some(AlertLevel::Critical),
            subtype: None,
            message: transition.alert.clone(),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        }),
    ]
}

// ---------------- relay_service ------------------

/// Relay timeout event.
pub(crate) fn relay_timeout_event(pending: &ExpiredRelayConfiguration) -> ServerEvent {
    let message = format!(
        "Configuration timed out for device {} on channel {}",
        pending.device_mac, pending.channel_number
    );
    ServerEvent::frontend(FrontendMessagePayload::RelayConfigurationStatus {
        relay_id: pending.relay_id,
        controller_id: pending.controller_id.as_deref().map(str::to_owned),
        status: "failed".to_owned(),
        total: 1,
        completed: 0,
        errors: vec![message.clone()],
        message: Some(message),
    })
}
