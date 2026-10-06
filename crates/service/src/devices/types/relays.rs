use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::pagination::PaginationResponse;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayRegistration {
    pub serial_number: String,
    #[serde(default)]
    pub state: BTreeMap<String, i32>,
    #[serde(default)]
    pub channels: Vec<RelayChannelRegistration>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayChannelRegistration {
    pub relay_channel_id: Option<Uuid>,
    #[schema(minimum = 1)]
    pub channel_number: i32,
    pub device_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Relay {
    pub id: Uuid,
    pub serial_number: String,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    pub device_controller_id: String,
    pub state: String,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayDeviceSummary {
    pub device_id: String,
    pub mac_address: String,
    pub device_name: String,
    pub ip_address: String,
    pub device_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayChannelRecord {
    pub id: Uuid,
    pub relay_id: Uuid,
    pub channel_number: i32,
    pub device_id: Option<String>,
    pub state: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub deleted_at: Option<DateTime<Utc>>,
    pub device: Option<RelayDeviceSummary>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayRecord {
    pub id: Uuid,
    pub serial_number: String,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    pub device_controller_id: String,
    pub state: String,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub deleted_at: Option<DateTime<Utc>>,
    pub relay_channels: Option<Vec<RelayChannelRecord>>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AvailableRelayDevicesQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub relay_id: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AvailableRelayDevice {
    pub device_id: String,
    pub mac_address: Option<String>,
    pub device_name: Option<String>,
    pub ip_address: Option<String>,
    pub device_type: Option<String>,
    pub device_family: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AvailableRelayDevicesResponse {
    pub devices: Vec<AvailableRelayDevice>,
    pub pagination: PaginationResponse,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LegacyRelayConfigurationResponse {
    #[schema(example = "Device configured with relay successfully")]
    pub message: String,
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelayConflictResponse {
    #[schema(example = false)]
    pub conflict: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayIdentityUpdateRequest {
    pub serial_number: Option<String>,
    pub vid_pid: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayIdentityUpdateResponse {
    pub message: &'static str,
    pub relay_id: Uuid,
    pub serial_number: String,
    pub vid_pid: String,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRelayConfiguration {
    pub id: Option<Uuid>,
    pub relay_id: Uuid,
    pub device_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRelayRemapRequest {
    pub relay_id: String,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRelayListQuery {
    pub controller_id: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRelayChannelListQuery {
    pub device_id: Option<String>,
    pub relay_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRelayConflictQuery {
    pub relay_id: Uuid,
    pub channel_number: i32,
    pub device_id: Option<String>,
    pub relay_channel_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayConflictState {
    pub device_conflict: bool,
    pub relay_conflict: bool,
    pub channel_conflict: bool,
}
