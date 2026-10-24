//! UART configure request/response types for `/uart/configure`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Request/response body type `UartConfigurationRequest` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UartConfigurationRequest {
    /// EdgeController identifier.
    pub controller_id: String,
    /// Stable device identifier (often MAC-derived).
    pub device_id: String,
    /// `UartConfigurationRequest.relay_id` used by device HTTP handlers.
    pub relay_id: Option<Uuid>,
    /// `UartConfigurationRequest.channel_id` used by device HTTP handlers.
    pub channel_id: Option<Uuid>,
    /// `UartConfigurationRequest.uart_vid_pid` used by device HTTP handlers.
    pub uart_vid_pid: String,
}

/// Request/response body type `UartConfigurationResponse` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UartConfigurationResponse {
    /// Device or interface MAC / UID identifier.
    pub mac: String,
    /// Hardware generation (e.g. 4 or 5) for IPL/flash paths.
    pub generation: u8,
    /// `UartConfigurationResponse.vid_pid` used by device HTTP handlers.
    pub vid_pid: String,
    /// `UartConfigurationResponse.tty` used by device HTTP handlers.
    pub tty: String,
    /// `UartConfigurationResponse.usb_serial` used by device HTTP handlers.
    pub usb_serial: Option<String>,
    /// `UartConfigurationResponse.interface` used by device HTTP handlers.
    pub interface: u8,
    /// `UartConfigurationResponse.topology` used by device HTTP handlers.
    pub topology: String,
    /// `UartConfigurationResponse.connection` used by device HTTP handlers.
    pub connection: String,
    /// `UartConfigurationResponse.verified` used by device HTTP handlers.
    pub verified: bool,
}
