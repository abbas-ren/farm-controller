use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStateAnalytics {
    pub total: i64,
    pub state_count: std::collections::BTreeMap<String, i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStateDetailedItem {
    pub device_id: String,
    pub device_type: String,
    pub state: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(transparent)]
pub struct DeviceStateDetailedAnalytics(
    pub std::collections::BTreeMap<String, Vec<DeviceStateDetailedItem>>,
);
