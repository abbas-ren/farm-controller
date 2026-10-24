//! Query/response types for `/api/v1/device/analytics/*` handlers.

use serde::Serialize;
use utoipa::ToSchema;

/// Request/response body type `DeviceStateAnalytics` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStateAnalytics {
    /// Total matching rows before pagination.
    pub total: i64,
    /// `DeviceStateAnalytics.state_count` used by device HTTP handlers.
    pub state_count: std::collections::BTreeMap<String, i64>,
}

/// Request/response body type `DeviceStateDetailedItem` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStateDetailedItem {
    /// Stable device identifier (often MAC-derived).
    pub device_id: String,
    /// Device type string used for build/artifact matching.
    pub device_type: String,
    /// Channel or device power/state value.
    pub state: String,
}

/// Request/response body type `DeviceStateDetailedAnalytics` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(transparent)]
pub struct DeviceStateDetailedAnalytics(
    /// `DeviceStateDetailedAnalytics.std` used by device HTTP handlers.
    pub std::collections::BTreeMap<String, Vec<DeviceStateDetailedItem>>,
);
