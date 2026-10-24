//! Legacy and modern relay inventory / channel wire types.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::pagination::PaginationResponse;

/// Request/response body type `RelayRegistration` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayRegistration {
    /// Relay hardware serial number.
    pub serial_number: String,
    /// Channel or device power/state value.
    #[serde(default)]
    pub state: BTreeMap<String, i32>,
    /// `RelayRegistration.channels` used by device HTTP handlers.
    #[serde(default)]
    pub channels: Vec<RelayChannelRegistration>,
}

/// Request/response body type `RelayChannelRegistration` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayChannelRegistration {
    /// `RelayChannelRegistration.relay_channel_id` used by device HTTP handlers.
    pub relay_channel_id: Option<Uuid>,
    /// Relay channel number.
    #[schema(minimum = 1)]
    pub channel_number: i32,
    /// Stable device identifier (often MAC-derived).
    pub device_id: Option<String>,
}

/// Request/response body type `Relay` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema, sqlx::FromRow, sea_orm::FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct Relay {
    /// Resource identifier.
    pub id: Uuid,
    /// Relay hardware serial number.
    pub serial_number: String,
    /// `Relay.vendor_id` used by device HTTP handlers.
    pub vendor_id: Option<String>,
    /// `Relay.product_id` used by device HTTP handlers.
    pub product_id: Option<String>,
    /// Persisted EdgeController primary key / UID.
    pub device_controller_id: String,
    /// Channel or device power/state value.
    pub state: String,
    /// Creation timestamp (RFC3339 or DB datetime).
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    /// `Relay.updated_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

/// Request/response body type `RelayDeviceSummary` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayDeviceSummary {
    /// Stable device identifier (often MAC-derived).
    pub device_id: String,
    /// Device MAC address as registered by the edge.
    pub mac_address: String,
    /// Human-readable device display name.
    pub device_name: String,
    /// Controller or device IPv4 address.
    pub ip_address: String,
    /// Device type string used for build/artifact matching.
    pub device_type: Option<String>,
}

/// Request/response body type `RelayChannelRecord` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayChannelRecord {
    /// Resource identifier.
    pub id: Uuid,
    /// `RelayChannelRecord.relay_id` used by device HTTP handlers.
    pub relay_id: Uuid,
    /// Relay channel number.
    pub channel_number: i32,
    /// Stable device identifier (often MAC-derived).
    pub device_id: Option<String>,
    /// Channel or device power/state value.
    pub state: Option<String>,
    /// `RelayChannelRecord.created_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    /// `RelayChannelRecord.updated_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
    /// `RelayChannelRecord.deleted_at` used by device HTTP handlers.
    #[schema(value_type = Option<String>, format = DateTime)]
    pub deleted_at: Option<DateTime<Utc>>,
    /// `RelayChannelRecord.device` used by device HTTP handlers.
    pub device: Option<RelayDeviceSummary>,
}

/// Request/response body type `RelayRecord` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayRecord {
    /// Resource identifier.
    pub id: Uuid,
    /// Relay hardware serial number.
    pub serial_number: String,
    /// `RelayRecord.vendor_id` used by device HTTP handlers.
    pub vendor_id: Option<String>,
    /// `RelayRecord.product_id` used by device HTTP handlers.
    pub product_id: Option<String>,
    /// Persisted EdgeController primary key / UID.
    pub device_controller_id: String,
    /// Channel or device power/state value.
    pub state: String,
    /// Creation timestamp (RFC3339 or DB datetime).
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    /// `RelayRecord.updated_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
    /// `RelayRecord.deleted_at` used by device HTTP handlers.
    #[schema(value_type = Option<String>, format = DateTime)]
    pub deleted_at: Option<DateTime<Utc>>,
    /// `RelayRecord.relay_channels` used by device HTTP handlers.
    pub relay_channels: Option<Vec<RelayChannelRecord>>,
}

/// Request/response body type `AvailableRelayDevicesQuery` for device HTTP handlers.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AvailableRelayDevicesQuery {
    /// 1-based page number for paginated lists.
    pub page: Option<u32>,
    /// Maximum rows to return (capped by MAX_PAGE_SIZE).
    pub limit: Option<u32>,
    /// `AvailableRelayDevicesQuery.relay_id` used by device HTTP handlers.
    pub relay_id: Option<Uuid>,
}

/// Request/response body type `AvailableRelayDevice` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AvailableRelayDevice {
    /// Stable device identifier (often MAC-derived).
    pub device_id: String,
    /// Device MAC address as registered by the edge.
    pub mac_address: Option<String>,
    /// Human-readable device display name.
    pub device_name: Option<String>,
    /// Controller or device IPv4 address.
    pub ip_address: Option<String>,
    /// Device type string used for build/artifact matching.
    pub device_type: Option<String>,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    pub device_family: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AvailableRelayDevicesResponse {
    /// `AvailableRelayDevicesResponse.devices` used by device HTTP handlers.
    pub devices: Vec<AvailableRelayDevice>,
    /// `AvailableRelayDevicesResponse.pagination` used by device HTTP handlers.
    pub pagination: PaginationResponse,
}

/// Request/response body type `LegacyRelayConfigurationResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct LegacyRelayConfigurationResponse {
    /// Human-readable status or error text for the client.
    #[schema(example = "Device configured with relay successfully")]
    pub message: String,
    /// Payload body preserved for frontend compatibility.
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
}

/// Request/response body type `RelayConflictResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct RelayConflictResponse {
    /// `RelayConflictResponse.conflict` used by device HTTP handlers.
    #[schema(example = false)]
    pub conflict: bool,
}

/// Request/response body type `RelayIdentityUpdateRequest` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayIdentityUpdateRequest {
    /// Relay hardware serial number.
    pub serial_number: Option<String>,
    /// `RelayIdentityUpdateRequest.vid_pid` used by device HTTP handlers.
    pub vid_pid: String,
}

/// Request/response body type `RelayIdentityUpdateResponse` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayIdentityUpdateResponse {
    /// Human-readable status or error text for the client.
    pub message: &'static str,
    /// `RelayIdentityUpdateResponse.relay_id` used by device HTTP handlers.
    pub relay_id: Uuid,
    /// Relay hardware serial number.
    pub serial_number: String,
    /// `RelayIdentityUpdateResponse.vid_pid` used by device HTTP handlers.
    pub vid_pid: String,
}

/// Request/response body type `LegacyRelayConfiguration` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRelayConfiguration {
    /// Resource identifier.
    pub id: Option<Uuid>,
    /// `LegacyRelayConfiguration.relay_id` used by device HTTP handlers.
    pub relay_id: Uuid,
    /// Stable device identifier (often MAC-derived).
    pub device_id: Option<String>,
}

/// Request/response body type `LegacyRelayRemapRequest` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRelayRemapRequest {
    /// `LegacyRelayRemapRequest.relay_id` used by device HTTP handlers.
    pub relay_id: String,
}

/// Request/response body type `LegacyRelayListQuery` for device HTTP handlers.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRelayListQuery {
    /// EdgeController identifier.
    pub controller_id: Option<String>,
}

/// Request/response body type `LegacyRelayChannelListQuery` for device HTTP handlers.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRelayChannelListQuery {
    /// Stable device identifier (often MAC-derived).
    pub device_id: Option<String>,
    /// `LegacyRelayChannelListQuery.relay_id` used by device HTTP handlers.
    pub relay_id: Option<Uuid>,
}

/// Request/response body type `LegacyRelayConflictQuery` for device HTTP handlers.
#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRelayConflictQuery {
    /// `LegacyRelayConflictQuery.relay_id` used by device HTTP handlers.
    pub relay_id: Uuid,
    /// Relay channel number.
    pub channel_number: i32,
    /// Stable device identifier (often MAC-derived).
    pub device_id: Option<String>,
    /// `LegacyRelayConflictQuery.relay_channel_id` used by device HTTP handlers.
    pub relay_channel_id: Option<Uuid>,
}

/// Request/response body type `RelayConflictState` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayConflictState {
    /// `RelayConflictState.device_conflict` used by device HTTP handlers.
    pub device_conflict: bool,
    /// `RelayConflictState.relay_conflict` used by device HTTP handlers.
    pub relay_conflict: bool,
    /// `RelayConflictState.channel_conflict` used by device HTTP handlers.
    pub channel_conflict: bool,
}
