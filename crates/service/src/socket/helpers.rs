use axum::http::{HeaderMap, HeaderValue, header};
use socketioxide::{
    SocketIo,
    extract::{SocketRef, TryData},
    layer::SocketIoLayer,
};

use crate::{
    devices::{ControllerHeartbeat, DeviceHeartbeatResult}, socket::{
        constants::BROWSER_BEARER_PREFIX, types::{
            farm::{BrowserAuthorization, EventHub, NativeSocketQuery, ServerEvent}, frontend::{
                AlertLevel, CpuMetrics, DiskMetrics, FrontendMessagePayload, MemoryMetrics, Metrics, NetworkMetrics,
            },
        },
    },
};

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

pub(crate) fn socket_io_layer(event_hub: &EventHub) -> SocketIoLayer {
    let (layer, socket_io) = SocketIo::builder()
        .max_payload(1024 * 1024)
        .max_buffer_size(256)
        .build_layer();
    socket_io.ns("/", socket_io_connect);
    event_hub.install_socket_io(socket_io);
    layer
}

async fn socket_io_connect(socket: SocketRef, TryData(auth): TryData<serde_json::Value>) {
    if let Ok(auth) = auth {
        // Validate/authenticate `auth` here if required.
        tracing::debug!(socket_id = %socket.id, "Socket.IO client connected");
    } else {
        tracing::warn!(
            socket_id = %socket.id,
            "Socket.IO connection received invalid auth data"
        );
    }
}

pub(crate) fn normalize_controller_id(value: &str) -> String {
    value.trim().replace([':', '-'], "").to_lowercase()
}

pub(crate) fn controller_heartbeat_payload(
    controller_id: String,
    heartbeat: &ControllerHeartbeat,
) -> FrontendMessagePayload {
    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp(heartbeat.timestamp, 0)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| heartbeat.timestamp.to_string());
    FrontendMessagePayload::EdgeControllerPing {
        controller_id,
        timestamp,
        metrics: Metrics {
            cpu: CpuMetrics {
                current: heartbeat.cpu_current,
                total: heartbeat.cpu_total,
                usage_percent: heartbeat.cpu_usage_percent,
            },
            memory: MemoryMetrics {
                used: heartbeat.memory_used,
                total: heartbeat.memory_total,
                usage_percent: heartbeat.memory_usage_percent,
            },
            network: NetworkMetrics {
                upload: heartbeat.network_upload,
                download: heartbeat.network_download,
            },
            disk: DiskMetrics {
                used: heartbeat.disk_used,
                total: heartbeat.disk_total,
                usage_percent: heartbeat.disk_usage_percent,
            },
            timestamp,
        },
    }
}

pub(crate) fn device_heartbeat_events(
    device_id: &str,
    mut result: DeviceHeartbeatResult,
    timestamp: &str,
) -> Vec<ServerEvent> {
    let mut events = Vec::new();

    if result.state_changed {
        events.push(ServerEvent::frontend(
            FrontendMessagePayload::DeviceStateUpdate {
                device_id: device_id.to_owned(),
                state: result.state.clone(),
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
                changes: serde_json::to_value(&result.interface_changes)
                    .unwrap_or(serde_json::Value::Null),
                heartbeat_data: serde_json::to_value(&result.heartbeat_data)
                    .unwrap_or(serde_json::Value::Null),
                timestamp: timestamp.to_owned(),
            },
        ));
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
        FrontendMessagePayload::EdgeHeartbeat {
            device_id: device_id.to_owned(),
            timestamp: timestamp.to_owned(),
        },
    ));

    // Migrate post_update_events at their source to GeneralPayload variants.
    // Do not forward their old arbitrary event names into the new typed protocol.

    events
}
