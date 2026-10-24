//! Device registration request body and related wire types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{controllers::DeviceController, relays::RelayRegistration};

/// Request/response body type `ControllerRegistration` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControllerRegistration {
    /// Device MAC address as registered by the edge.
    pub mac_address: String,
    /// Controller or device IPv4 address.
    pub ip_address: String,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    #[serde(default)]
    pub device_family: Option<String>,
    /// Relay inventory reported by the EdgeController.
    #[serde(default)]
    pub relays: Vec<RelayRegistration>,
    /// Alternate unique id supplied by the controller.
    #[serde(default)]
    pub uid: Option<String>,
}

/// PURPOSE: Serde default device state string for registration.
/// USE: Deserialization of DeviceRegistration.
fn default_device_state() -> String {
    let _api_trace = crate::observability::ApiTrace::enter("devices::types::registration", "default_device_state");
    "free".to_owned()
}

/// PURPOSE: Serde default device status string for registration.
/// USE: Deserialization of DeviceRegistration.
fn default_device_status() -> String {
    let _api_trace = crate::observability::ApiTrace::enter("devices::types::registration", "default_device_status");
    "requested".to_owned()
}

/// PURPOSE: Serde default heartbeat timeout for registration.
/// USE: Deserialization of DeviceRegistration.
fn default_heartbeat_timeout() -> i64 {
    let _api_trace = crate::observability::ApiTrace::enter("devices::types::registration", "default_heartbeat_timeout");
    5
}

/// Request/response body type `DeviceRegistration` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRegistration {
    /// Device MAC address as registered by the edge.
    pub mac_address: String,
    /// Controller or device IPv4 address.
    pub ip_address: String,
    /// Human-readable device display name.
    pub device_name: String,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    #[serde(default)]
    pub device_family: Option<String>,
    /// Device type string used for build/artifact matching.
    #[serde(default)]
    pub device_type: Option<String>,
    /// Firmware/build inventory identifier.
    #[serde(default)]
    pub build_id: Option<String>,
    /// `DeviceRegistration.timeout` used by device HTTP handlers.
    #[serde(default = "default_heartbeat_timeout")]
    pub timeout: i64,
    /// `DeviceRegistration.interfaces` used by device HTTP handlers.
    #[serde(default)]
    pub interfaces: BTreeMap<String, Vec<String>>,
    /// `DeviceRegistration.software_version` used by device HTTP handlers.
    #[serde(default)]
    pub software_version: Option<String>,
    /// `DeviceRegistration.nfs_path` used by device HTTP handlers.
    #[serde(default)]
    pub nfs_path: Option<String>,
    /// Channel or device power/state value.
    #[serde(default = "default_device_state")]
    pub state: String,
    /// Outcome or lifecycle status for this record.
    #[serde(default = "default_device_status")]
    pub status: String,
}

/// Request/response body type `ControllerRegistrationResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct ControllerRegistrationResponse {
    /// Whether the legacy API considers the operation successful.
    pub success: bool,
    /// Payload body preserved for frontend compatibility.
    pub data: DeviceController,
}
