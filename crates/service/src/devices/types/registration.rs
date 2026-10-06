use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{controllers::DeviceController, relays::RelayRegistration};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControllerRegistration {
    pub mac_address: String,
    pub ip_address: String,
    #[serde(default)]
    pub device_family: Option<String>,
    #[serde(default)]
    pub relays: Vec<RelayRegistration>,
    #[serde(default)]
    pub uid: Option<String>,
}

fn default_device_state() -> String {
    "free".to_owned()
}

fn default_device_status() -> String {
    "requested".to_owned()
}

fn default_heartbeat_timeout() -> i64 {
    5
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRegistration {
    pub mac_address: String,
    pub ip_address: String,
    pub device_name: String,
    #[serde(default)]
    pub device_family: Option<String>,
    #[serde(default)]
    pub device_type: Option<String>,
    #[serde(default)]
    pub build_id: Option<String>,
    #[serde(default = "default_heartbeat_timeout")]
    pub timeout: i64,
    #[serde(default)]
    pub interfaces: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub software_version: Option<String>,
    #[serde(default)]
    pub nfs_path: Option<String>,
    #[serde(default = "default_device_state")]
    pub state: String,
    #[serde(default = "default_device_status")]
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ControllerRegistrationResponse {
    pub success: bool,
    pub data: DeviceController,
}
