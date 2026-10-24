//! EdgeController registration, edit, and list wire types.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::relays::Relay;

/// Request/response body type `DeviceController` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceController {
    /// Persisted EdgeController primary key / UID.
    pub device_controller_id: String,
    /// Device MAC address as registered by the edge.
    pub mac_address: String,
    /// Controller or device IPv4 address.
    pub ip_address: String,
    /// Display name for the resource.
    pub name: Option<String>,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    pub device_family: Option<String>,
    /// Outcome or lifecycle status for this record.
    pub status: String,
    /// Channel or device power/state value.
    pub state: String,
    /// `DeviceController.created_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    /// `DeviceController.updated_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
    /// Relay inventory reported by the EdgeController.
    pub relays: Vec<Relay>,
}

/// Request/response body type `DeviceTypesQuery` for device HTTP handlers.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTypesQuery {
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    pub device_family: Option<String>,
}

/// Request/response body type `DeviceListQuery` for device HTTP handlers.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceListQuery {
    /// `DeviceListQuery.sort_by` used by device HTTP handlers.
    pub sort_by: Option<String>,
    /// `DeviceListQuery.desc` used by device HTTP handlers.
    pub desc: Option<bool>,
    /// Free-text filter applied by the handler/repository.
    pub search: Option<String>,
    /// 1-based page number for paginated lists.
    pub page: Option<u32>,
    /// Maximum rows to return (capped by MAX_PAGE_SIZE).
    pub limit: Option<u32>,
    /// `DeviceListQuery.filter_by` used by device HTTP handlers.
    pub filter_by: Option<String>,
    /// `DeviceListQuery.filter` used by device HTTP handlers.
    pub filter: Option<String>,
}

/// Request/response body type `DeviceCsvQuery` for device HTTP handlers.
#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCsvQuery {
    /// Outcome or lifecycle status for this record.
    pub status: Option<String>,
    /// `DeviceCsvQuery.from_date` used by device HTTP handlers.
    pub from_date: Option<String>,
    /// `DeviceCsvQuery.to_date` used by device HTTP handlers.
    pub to_date: Option<String>,
    /// Free-text filter applied by the handler/repository.
    pub search: Option<String>,
}

/// Request/response body type `DeviceList` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceList {
    /// Payload body preserved for frontend compatibility.
    pub data: Vec<serde_json::Value>,
    /// `DeviceList.total_pages` used by device HTTP handlers.
    pub total_pages: u64,
    /// `DeviceList.current_page` used by device HTTP handlers.
    pub current_page: u32,
    /// `DeviceList.total_devices` used by device HTTP handlers.
    pub total_devices: u64,
    /// `DeviceList.requested_count` used by device HTTP handlers.
    pub requested_count: u64,
    /// `DeviceList.device_timers` used by device HTTP handlers.
    pub device_timers: BTreeMap<String, i64>,
    /// `DeviceList.device_timeouts` used by device HTTP handlers.
    pub device_timeouts: BTreeMap<String, i64>,
    /// `DeviceList.state_count` used by device HTTP handlers.
    pub state_count: BTreeMap<String, u64>,
}

/// Enumerated wire value `DeviceAction` for device HTTP handlers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeviceAction {
    Approved,
    Declined,
}

/// Request/response body type `DeviceActionRequest` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeviceActionRequest {
    /// `DeviceActionRequest.action` used by device HTTP handlers.
    pub action: DeviceAction,
}

/// Request/response body type `HeartbeatTimeoutRequest` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatTimeoutRequest {
    /// Stable device identifier (often MAC-derived).
    pub device_id: String,
    /// `HeartbeatTimeoutRequest.value` used by device HTTP handlers.
    pub value: i64,
}

/// Request/response body type `ControllerEditRequest` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControllerEditRequest {
    /// Display name for the resource.
    pub name: Option<String>,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    pub device_family: Option<String>,
    /// Controller or device IPv4 address.
    pub ip_address: Option<String>,
}

/// Request/response body type `ControllerListQuery` for device HTTP handlers.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControllerListQuery {
    /// Free-text filter applied by the handler/repository.
    pub search: Option<String>,
    /// Outcome or lifecycle status for this record.
    pub status: Option<String>,
    /// Channel or device power/state value.
    pub state: Option<String>,
    /// `ControllerListQuery.sort_by` used by device HTTP handlers.
    pub sort_by: Option<String>,
    /// `ControllerListQuery.desc` used by device HTTP handlers.
    pub desc: Option<bool>,
    /// 1-based page number for paginated lists.
    pub page: Option<u32>,
    /// Maximum rows to return (capped by MAX_PAGE_SIZE).
    pub limit: Option<u32>,
}

/// Request/response body type `ControllerList` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControllerList {
    /// Payload body preserved for frontend compatibility.
    pub data: Vec<serde_json::Value>,
    /// `ControllerList.total_pages` used by device HTTP handlers.
    pub total_pages: u64,
    /// `ControllerList.current_page` used by device HTTP handlers.
    pub current_page: u32,
    /// `ControllerList.total_count` used by device HTTP handlers.
    pub total_count: u64,
    /// `ControllerList.summary` used by device HTTP handlers.
    pub summary: serde_json::Value,
}

/// Request/response body type `UserDeviceListQuery` for device HTTP handlers.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserDeviceListQuery {
    /// `UserDeviceListQuery.sort_by` used by device HTTP handlers.
    pub sort_by: Option<String>,
    /// `UserDeviceListQuery.desc` used by device HTTP handlers.
    pub desc: Option<bool>,
    /// Free-text filter applied by the handler/repository.
    pub search: Option<String>,
    /// 1-based page number for paginated lists.
    pub page: Option<u32>,
    /// Maximum rows to return (capped by MAX_PAGE_SIZE).
    pub limit: Option<u32>,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    pub device_family: Option<String>,
    /// `UserDeviceListQuery.show_all` used by device HTTP handlers.
    pub show_all: Option<bool>,
    /// `UserDeviceListQuery.filter_by` used by device HTTP handlers.
    pub filter_by: Option<String>,
}

/// Request/response body type `ControllerHeartbeat` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControllerHeartbeat {
    /// `ControllerHeartbeat.message_type` used by device HTTP handlers.
    #[serde(rename = "type")]
    pub message_type: String,
    /// Alternate unique id supplied by the controller.
    pub uid: String,
    /// Caller or device IPv4 address.
    pub ip: String,
    /// RFC3339 timestamp for the event or record.
    pub timestamp: i64,
    /// `ControllerHeartbeat.cpu_current` used by device HTTP handlers.
    pub cpu_current: String,
    /// `ControllerHeartbeat.cpu_total` used by device HTTP handlers.
    pub cpu_total: String,
    /// `ControllerHeartbeat.cpu_usage_percent` used by device HTTP handlers.
    pub cpu_usage_percent: f64,
    /// `ControllerHeartbeat.memory_used` used by device HTTP handlers.
    pub memory_used: String,
    /// `ControllerHeartbeat.memory_total` used by device HTTP handlers.
    pub memory_total: String,
    /// `ControllerHeartbeat.memory_usage_percent` used by device HTTP handlers.
    pub memory_usage_percent: f64,
    /// `ControllerHeartbeat.network_upload` used by device HTTP handlers.
    pub network_upload: String,
    /// `ControllerHeartbeat.network_download` used by device HTTP handlers.
    pub network_download: String,
    /// `ControllerHeartbeat.disk_used` used by device HTTP handlers.
    pub disk_used: String,
    /// `ControllerHeartbeat.disk_total` used by device HTTP handlers.
    pub disk_total: String,
    /// `ControllerHeartbeat.disk_usage_percent` used by device HTTP handlers.
    pub disk_usage_percent: f64,
}
