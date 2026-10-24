//! In-process `EventHub` and `ServerEvent` envelope published to farm sockets.

use serde::{Deserialize, Serialize};
use socketioxide::SocketIo;
use std::sync::OnceLock;
use utoipa::ToSchema;

/// Farm WebSocket event envelope: `type` string + JSON `payload` for UI listeners.
#[derive(Clone, Debug, Serialize)]
pub struct ServerEvent {
    /// Event name (serialized as `type`); e.g. `system_metrics_update`, device refreshes.
    #[serde(rename = "type")]
    pub event: String,
    /// Event-specific JSON body matching the legacy frontend contract.
    pub payload: serde_json::Value,
}

/// Process-wide broadcast hub plus optional Socket.IO handle for live emits.
pub struct EventHub {
    pub sender: tokio::sync::broadcast::Sender<ServerEvent>,
    pub socket_io: OnceLock<SocketIo>,
}

/// HTTP body for `send_user_message` companion endpoint.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UserMessageRequest {
    pub data: Option<serde_json::Value>,
}

/// Ack for user-message HTTP companion.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserMessageResponse {
    pub ok: bool,
}

/// Error body when event publish/auth fails on socket companion routes.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventErrorResponse {
    pub message: String,
}

/// Query string carried by native (non-browser) socket clients on connect.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeSocketQuery {
    pub device_controller_id: Option<String>,
    pub controller_id: Option<String>,
    pub device_id: Option<String>,
    pub test_id: Option<String>,
    pub client: Option<String>,
    pub user_name: Option<String>,
    pub admin_terminal: Option<bool>,
}

/// Browser socket authorization tier derived from the session cookie/token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserAuthorization {
    None,
    Authenticated,
    Admin,
}
