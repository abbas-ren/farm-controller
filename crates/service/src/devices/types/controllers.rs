use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::relays::Relay;

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceController {
    pub device_controller_id: String,
    pub mac_address: String,
    pub ip_address: String,
    pub name: Option<String>,
    pub device_family: Option<String>,
    pub status: String,
    pub state: String,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
    pub relays: Vec<Relay>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTypesQuery {
    pub device_family: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceListQuery {
    pub sort_by: Option<String>,
    pub desc: Option<bool>,
    pub search: Option<String>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub filter_by: Option<String>,
    pub filter: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCsvQuery {
    pub status: Option<String>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub search: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceList {
    pub data: Vec<serde_json::Value>,
    pub total_pages: u64,
    pub current_page: u32,
    pub total_devices: u64,
    pub requested_count: u64,
    pub device_timers: BTreeMap<String, i64>,
    pub device_timeouts: BTreeMap<String, i64>,
    pub state_count: BTreeMap<String, u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeviceAction {
    Approved,
    Declined,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeviceActionRequest {
    pub action: DeviceAction,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatTimeoutRequest {
    pub device_id: String,
    pub value: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControllerEditRequest {
    pub name: Option<String>,
    pub device_family: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControllerListQuery {
    pub search: Option<String>,
    pub status: Option<String>,
    pub state: Option<String>,
    pub sort_by: Option<String>,
    pub desc: Option<bool>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControllerList {
    pub data: Vec<serde_json::Value>,
    pub total_pages: u64,
    pub current_page: u32,
    pub total_count: u64,
    pub summary: serde_json::Value,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserDeviceListQuery {
    pub sort_by: Option<String>,
    pub desc: Option<bool>,
    pub search: Option<String>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub device_family: Option<String>,
    pub show_all: Option<bool>,
    pub filter_by: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControllerHeartbeat {
    #[serde(rename = "type")]
    pub message_type: String,
    pub uid: String,
    pub ip: String,
    pub timestamp: i64,
    pub cpu_current: String,
    pub cpu_total: String,
    pub cpu_usage_percent: f64,
    pub memory_used: String,
    pub memory_total: String,
    pub memory_usage_percent: f64,
    pub network_upload: String,
    pub network_download: String,
    pub disk_used: String,
    pub disk_total: String,
    pub disk_usage_percent: f64,
}
