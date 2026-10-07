use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UartConfigurationRequest {
    pub controller_id: String,
    pub device_id: String,
    pub relay_id: Option<Uuid>,
    pub channel_id: Option<Uuid>,
    pub uart_vid_pid: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UartConfigurationResponse {
    pub mac: String,
    pub generation: u8,
    pub vid_pid: String,
    pub tty: String,
    pub usb_serial: Option<String>,
    pub interface: u8,
    pub topology: String,
    pub connection: String,
    pub verified: bool,
}
