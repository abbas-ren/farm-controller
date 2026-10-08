#[cfg(test)]
pub mod tests;

mod terminal;
mod web_cli;

use std::sync::{Arc, OnceLock};

use axum::{
    Json, Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket, close_code},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use socketioxide::{
    SocketIo,
    extract::{Data, SocketRef, TryData},
    layer::SocketIoLayer,
};
use utoipa::ToSchema;

use crate::{
    devices::{ControllerHeartbeat, repository_types::DeviceHeartbeatResult},
    state::AppState,
};

const EDGE_SUBPROTOCOL: &str = "web-cli-protocol";
const ROOM_REGISTRATIONS: [(&str, &str, &str); 5] = [
    ("join:build", "leave:build", "build"),
    ("join:test", "leave:test", "test"),
    ("join:dashboard", "leave:dashboard", "dashboard"),
    ("join:device", "leave:device", "device"),
    (
        "join:device-controller",
        "leave:device-controller",
        "device-controller",
    ),
];

#[derive(Clone, Debug, Serialize)]
pub struct ServerEvent {
    pub event: String,
    pub payload: serde_json::Value,
    pub room: Option<String>,
}

impl ServerEvent {
    pub(crate) fn alert(subtype: &str, message: serde_json::Value) -> Self {
        Self {
            event: "alert".to_owned(),
            payload: serde_json::json!({
                "type": "alert",
                "subtype": subtype,
                "message": message,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            }),
            room: None,
        }
    }
}

pub(crate) fn build_performance_events(build_id: &str) -> [ServerEvent; 3] {
    [
        "dashboard:admin".to_owned(),
        "dashboard:user".to_owned(),
        format!("build:{build_id}"),
    ]
    .map(|room| ServerEvent {
        event: "build_performance_update".to_owned(),
        payload: serde_json::json!({"buildId": build_id}),
        room: Some(room),
    })
}

pub struct EventHub {
    sender: tokio::sync::broadcast::Sender<ServerEvent>,
    socket_io: OnceLock<SocketIo>,
}

impl Default for EventHub {
    fn default() -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(256);
        Self {
            sender,
            socket_io: OnceLock::new(),
        }
    }
}

impl EventHub {
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ServerEvent> {
        self.sender.subscribe()
    }

    fn install_socket_io(&self, socket_io: SocketIo) {
        let _ = self.socket_io.set(socket_io);
    }

    pub fn publish(&self, event: ServerEvent) -> Result<(), String> {
        if let Some(socket_io) = self.socket_io.get() {
            for socket in socket_io.sockets() {
                if event.room.as_ref().is_some_and(|room| {
                    !socket
                        .rooms()
                        .iter()
                        .any(|candidate| candidate.as_ref() == room)
                }) {
                    continue;
                }
                if let Err(error) = socket.emit(&event.event, &event.payload) {
                    tracing::debug!(%error, event = event.event, "Socket.IO client emit failed");
                }
            }
        }
        let _ = self.sender.send(event);
        Ok(())
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UserMessageRequest {
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserMessageResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventErrorResponse {
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeSocketQuery {
    device_controller_id: Option<String>,
    controller_id: Option<String>,
    device_id: Option<String>,
    test_id: Option<String>,
    client: Option<String>,
    user_name: Option<String>,
    admin_terminal: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BrowserAuthorization {
    None,
    Authenticated,
    Admin,
}

fn browser_authorization(query: &NativeSocketQuery) -> Result<BrowserAuthorization, ()> {
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

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ws", get(upgrade))
        .route("/api/v1/device/ws/send/message", post(send_user_message))
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
    if let Ok(auth) = auth
        && let Some(user_id) = auth
            .get("userId")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        socket.join(format!("user:{user_id}"));
    }
    for (join, leave, prefix) in ROOM_REGISTRATIONS {
        register_room(&socket, join, leave, prefix);
    }
}

fn register_room(
    socket: &SocketRef,
    join_event: &'static str,
    leave_event: &'static str,
    prefix: &'static str,
) {
    socket.on(
        join_event,
        async move |socket: SocketRef, Data::<String>(id)| {
            let id = id.trim();
            if !id.is_empty() {
                socket.join(format!("{prefix}:{id}"));
            }
        },
    );
    socket.on(
        leave_event,
        async move |socket: SocketRef, Data::<String>(id)| {
            let id = id.trim();
            if !id.is_empty() {
                socket.leave(format!("{prefix}:{id}"));
            }
        },
    );
}

#[utoipa::path(post, path = "/api/v1/device/ws/send/message", tag = "Events", summary = "Dispatch a public user event", description = "Publishes the legacy `user` Socket.IO event with `{message,timestamp}` to connected browser clients.", request_body = UserMessageRequest, responses((status = 200, description = "Event dispatched", body = UserMessageResponse), (status = 500, description = "Event dispatch failed", body = EventErrorResponse)))]
pub(crate) async fn send_user_message(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UserMessageRequest>,
) -> Response {
    let event = ServerEvent {
        event: "user".to_owned(),
        payload: serde_json::json!({
            "message": request.data.unwrap_or(serde_json::Value::Null),
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }),
        room: None,
    };
    match state.event_publisher.publish(event) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(error) => {
            tracing::error!(%error, "failed to dispatch user event");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"message": error})),
            )
                .into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/ws",
    tag = "Events",
    summary = "Upgrade a native FarmController WebSocket",
    description = "Selects controller heartbeat, device heartbeat, native test-result, authenticated frontend terminal, or authenticated WebCLI mode from query parameters. Controller clients negotiate the web-cli-protocol subprotocol.",
    params(
        ("deviceControllerId" = Option<String>, Query, description = "EdgeController heartbeat identity"),
        ("controllerId" = Option<String>, Query, description = "Legacy controller identity alias"),
        ("deviceId" = Option<String>, Query, description = "Device identity; pair with testId for native results"),
        ("testId" = Option<String>, Query, description = "Active native test execution identity"),
        ("client" = Option<String>, Query, description = "Authenticated browser mode: frontend or cli"),
        ("userName" = Option<String>, Query, description = "Required display name for cli mode"),
        ("adminTerminal" = Option<bool>, Query, description = "Require the admin role and permit the FarmController host terminal")
    ),
    responses(
        (status = 101, description = "WebSocket protocol upgraded"),
        (status = 400, description = "No valid mode parameters supplied"),
        (status = 401, description = "Browser token or test session is invalid"),
        (status = 503, description = "Persistence required by the selected mode is unavailable")
    )
)]
pub(crate) async fn upgrade(
    State(state): State<Arc<AppState>>,
    Query(query): Query<NativeSocketQuery>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    let browser_authorization = match browser_authorization(&query) {
        Ok(authorization) => authorization,
        Err(()) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let controller_id = query
        .device_controller_id
        .or(query.controller_id)
        .unwrap_or_default();
    if !controller_id.trim().is_empty() {
        return websocket
            .protocols([EDGE_SUBPROTOCOL])
            .on_upgrade(move |socket| controller_session(state, controller_id, socket));
    }
    let browser_client = browser_authorization != BrowserAuthorization::None;
    let required_role = (browser_authorization == BrowserAuthorization::Admin).then_some("admin");
    if browser_client
        && crate::auth::authorize_request(&state, &headers, required_role)
            .await
            .is_err()
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let authenticated_user_id = browser_client
        .then(|| crate::auth::token_subject(&headers))
        .flatten();
    if query.client.as_deref() == Some("frontend") {
        let allow_farm_host = browser_authorization == BrowserAuthorization::Admin;
        return websocket
            .on_upgrade(move |socket| terminal::session(state, socket, allow_farm_host));
    }
    if query.client.as_deref() == Some("cli")
        && let (Some(user_id), Some(user_name)) = (authenticated_user_id, query.user_name)
        && !user_id.trim().is_empty()
        && !user_name.trim().is_empty()
    {
        return websocket
            .on_upgrade(move |socket| web_cli::session(state, user_id, user_name, socket));
    }
    if let (Some(test_id), Some(device_id)) = (query.test_id, query.device_id.clone()) {
        let pool = match state.database.as_ref() {
            Some(database) => database.pool(),
            None => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
        return match crate::test_results::validate_session(pool, &test_id, &device_id).await {
            Ok(()) => websocket
                .on_upgrade(move |socket| test_result_session(state, test_id, device_id, socket)),
            Err(error) => {
                tracing::warn!(%error, %test_id, %device_id, "rejected native test result session");
                StatusCode::UNAUTHORIZED.into_response()
            }
        };
    }
    let device_id = query.device_id.unwrap_or_default();
    if !device_id.trim().is_empty() {
        return websocket.on_upgrade(move |socket| device_session(state, device_id, socket));
    }
    (
        StatusCode::BAD_REQUEST,
        "deviceControllerId or deviceId is required",
    )
        .into_response()
}

async fn test_result_session(
    state: Arc<AppState>,
    test_id: String,
    device_id: String,
    mut socket: WebSocket,
) {
    tracing::info!(%test_id, %device_id, "test result WebSocket connected");
    while let Some(message) = socket.recv().await {
        let case_id = match message {
            Ok(Message::Text(text)) => text.trim().parse::<i64>().ok(),
            Ok(Message::Binary(bytes)) => std::str::from_utf8(&bytes)
                .ok()
                .and_then(|text| text.trim().parse::<i64>().ok()),
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(bytes)) => {
                if socket.send(Message::Pong(bytes)).await.is_err() {
                    break;
                }
                continue;
            }
            Ok(Message::Pong(_)) => continue,
            Err(error) => {
                tracing::warn!(%error, %test_id, %device_id, "test result WebSocket read failed");
                break;
            }
        };
        let Some(case_id) = case_id.filter(|case_id| *case_id > 0) else {
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: close_code::POLICY,
                    reason: "test case id must be a positive integer".into(),
                })))
                .await;
            break;
        };
        match crate::test_results::process_case(&state, &test_id, &device_id, case_id).await {
            Ok(result) => crate::test_results::sync_catalog(&state, &result).await,
            Err(error) => {
                tracing::warn!(%error, %test_id, %device_id, case_id, "failed to process native test result");
                let _ = socket
                    .send(Message::Close(Some(CloseFrame {
                        code: close_code::ERROR,
                        reason: "failed to process test result".into(),
                    })))
                    .await;
                break;
            }
        }
    }
    tracing::info!(%test_id, %device_id, "test result WebSocket disconnected");
}

async fn controller_session(state: Arc<AppState>, controller_id: String, mut socket: WebSocket) {
    tracing::info!(controller_id, "device controller WebSocket connected");
    while let Some(message) = socket.recv().await {
        let bytes = match message {
            Ok(Message::Binary(bytes)) => bytes,
            Ok(Message::Text(text)) => text.as_bytes().to_vec().into(),
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Err(error) => {
                tracing::warn!(%error, controller_id, "device controller WebSocket read failed");
                break;
            }
        };
        if let Err(error) = process_heartbeat(&state, &controller_id, &bytes).await {
            tracing::warn!(%error, controller_id, "invalid device controller heartbeat");
        }
    }
    tracing::info!(controller_id, "device controller WebSocket disconnected");
}

async fn device_session(state: Arc<AppState>, device_id: String, mut socket: WebSocket) {
    tracing::info!(device_id, "device WebSocket connected");
    while let Some(message) = socket.recv().await {
        let bytes = match message {
            Ok(Message::Binary(bytes)) => bytes,
            Ok(Message::Text(text)) => text.as_bytes().to_vec().into(),
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Err(error) => {
                tracing::warn!(%error, device_id, "device WebSocket read failed");
                break;
            }
        };
        if let Err(error) = process_device_heartbeat(&state, &device_id, &bytes).await {
            tracing::warn!(%error, device_id, "invalid device heartbeat");
        }
    }
    tracing::info!(device_id, "device WebSocket disconnected");
}

async fn process_device_heartbeat(
    state: &AppState,
    device_id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let heartbeat: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if heartbeat.get("type").and_then(serde_json::Value::as_str) != Some("heartbeat") {
        return Ok(());
    }
    let repository = state
        .device_repository
        .as_ref()
        .ok_or_else(|| "device persistence is unavailable".to_owned())?;
    let result = repository
        .record_device_heartbeat(device_id, &heartbeat)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "approved device was not found".to_owned())?;
    for cancellation in &result.deferred_cancellations {
        crate::devices::test_execution_handlers::complete_deferred_cancellation(
            state,
            cancellation,
        )
        .await;
    }
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    for event in device_heartbeat_events(device_id, result, &timestamp) {
        state.event_publisher.publish(event)?;
    }
    Ok(())
}

fn device_heartbeat_events(
    device_id: &str,
    mut result: DeviceHeartbeatResult,
    timestamp: &str,
) -> Vec<ServerEvent> {
    let mut events = Vec::new();
    if result.state_changed {
        events.push(ServerEvent {
            event: "device_state_update".to_owned(),
            payload: serde_json::json!({
                "deviceId": device_id,
                "state": result.state,
                "upgrading": false,
                "flashing": false,
                "changedAt": timestamp,
            }),
            room: None,
        });
    }
    if !result.interface_changes.is_empty() {
        events.push(ServerEvent {
            event: "device:interface-update".to_owned(),
            payload: serde_json::json!({
                "deviceId": device_id,
                "changes": result.interface_changes,
                "heartbeatData": result.heartbeat_data,
                "timestamp": timestamp,
            }),
            room: Some(format!("device:{device_id}")),
        });
    }
    events.extend(
        result
            .alerts
            .into_iter()
            .map(|alert| ServerEvent::alert("device-alert", alert)),
    );
    events.push(ServerEvent {
        event: "device:ping".to_owned(),
        payload: serde_json::json!({
            "deviceId": device_id,
            "timestamp": timestamp,
        }),
        room: Some(format!("device:{device_id}")),
    });
    events.extend(
        result
            .post_update_events
            .drain(..)
            .map(|event| ServerEvent {
                event: event.event,
                payload: event.payload,
                room: event.room,
            }),
    );
    events
}

async fn process_heartbeat(
    state: &AppState,
    controller_id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let heartbeat: ControllerHeartbeat =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if heartbeat.message_type != "heartbeat" {
        return Ok(());
    }
    let normalized_expected = normalize_controller_id(controller_id);
    let normalized_payload = normalize_controller_id(&heartbeat.uid);
    if normalized_expected != normalized_payload {
        return Err("heartbeat uid does not match the WebSocket identity".to_owned());
    }
    let repository = state
        .device_repository
        .as_ref()
        .ok_or_else(|| "device persistence is unavailable".to_owned())?;
    let result = repository
        .record_controller_heartbeat(&heartbeat)
        .await
        .map_err(|error| error.to_string())?;
    if result.state_changed {
        state.event_publisher.publish(ServerEvent {
            event: "device_controller_changed".to_owned(),
            payload: serde_json::json!({
                "action": "updated",
                "controllerId": normalized_payload,
            }),
            room: None,
        })?;
    }
    if let Some(alert) = result.alert {
        state
            .event_publisher
            .publish(ServerEvent::alert("device-alert", alert))?;
    }
    state
        .event_publisher
        .publish(controller_heartbeat_event(normalized_payload, &heartbeat))
}

fn controller_heartbeat_event(
    controller_id: String,
    heartbeat: &ControllerHeartbeat,
) -> ServerEvent {
    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp(heartbeat.timestamp, 0)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| heartbeat.timestamp.to_string());
    ServerEvent {
        event: "device_controller:ping".to_owned(),
        payload: serde_json::json!({
            "controllerId": controller_id,
            "timestamp": timestamp,
            "metrics": {
                "cpu": {
                    "current": heartbeat.cpu_current,
                    "total": heartbeat.cpu_total,
                    "usagePercent": heartbeat.cpu_usage_percent,
                },
                "memory": {
                    "used": heartbeat.memory_used,
                    "total": heartbeat.memory_total,
                    "usagePercent": heartbeat.memory_usage_percent,
                },
                "network": {
                    "upload": heartbeat.network_upload,
                    "download": heartbeat.network_download,
                },
                "disk": {
                    "used": heartbeat.disk_used,
                    "total": heartbeat.disk_total,
                    "usagePercent": heartbeat.disk_usage_percent,
                },
                "timestamp": timestamp,
            },
        }),
        room: Some(format!("device-controller:{controller_id}")),
    }
}

fn normalize_controller_id(value: &str) -> String {
    value.trim().replace([':', '-'], "").to_lowercase()
}
