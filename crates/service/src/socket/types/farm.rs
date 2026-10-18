use serde::{Deserialize, Serialize};
use socketioxide::SocketIo;
use std::sync::OnceLock;
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize)]
pub struct ServerEvent {
    #[serde(rename = "type")]
    pub event: String,
    pub payload: serde_json::Value,
}

pub struct EventHub {
    pub sender: tokio::sync::broadcast::Sender<ServerEvent>,
    pub socket_io: OnceLock<SocketIo>,
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
    pub device_controller_id: Option<String>,
    pub controller_id: Option<String>,
    pub device_id: Option<String>,
    pub test_id: Option<String>,
    pub client: Option<String>,
    pub user_name: Option<String>,
    pub admin_terminal: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserAuthorization {
    None,
    Authenticated,
    Admin,
}
