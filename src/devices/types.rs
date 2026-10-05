use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::error::ErrorResponse;

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
pub struct DeviceRegistrationResponse {
    pub success: bool,
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceDataResponse {
    pub success: bool,
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceHeartbeatResponse {
    pub success: bool,
    #[schema(value_type = Option<Object>, nullable = true)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceTopologyResponse {
    pub success: bool,
    #[schema(value_type = Vec<Object>)]
    pub data: Vec<serde_json::Value>,
}

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

#[derive(Debug, Serialize, ToSchema)]
pub struct ControllerRegistrationResponse {
    pub success: bool,
    pub data: DeviceController,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CallbackStatus {
    Success,
    Failure,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MappingCallback {
    #[schema(example = "aabbccddeeff")]
    pub mac: String,
    pub status: CallbackStatus,
    #[schema(example = json!({"uart": "/dev/ttyUSB0", "power": "1"}))]
    pub tty_entry: Option<serde_json::Value>,
    #[schema(example = "192.0.2.10")]
    pub ip: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct TtyEntry {
    pub uart: String,
    pub power: String,
}

#[derive(Debug, Deserialize)]
pub struct FlashConfirmationQuery {
    pub status: CallbackStatus,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestCompletionQuery {
    pub test_id: String,
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

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildsQuery {
    pub device_type: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildListQuery {
    pub search: Option<String>,
    pub device_type: Option<String>,
    pub device_family: Option<String>,
    pub flagged: Option<String>,
    pub build_version: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildList {
    pub builds: Vec<serde_json::Value>,
    pub total_count: u64,
    pub current_page: u64,
    pub total_pages: u64,
    pub requested_count: u64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildFilters {
    pub device_types: Vec<String>,
    pub device_families: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildUploadInitRequest {
    #[schema(minimum = 1, maximum = 5, example = 1)]
    pub file_count: i32,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildUploadInitResponse {
    #[schema(example = "8e8f632f-18ee-469f-a472-79245f37c842")]
    pub upload_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildUploadResponse {
    #[schema(example = "Upload successful")]
    pub message: String,
    #[schema(example = "8e8f632f-18ee-469f-a472-79245f37c842")]
    pub upload_id: String,
    #[schema(example = "RZG2L__v1.2.3.zip")]
    pub filename: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildFlagResponse {
    pub id: String,
    pub is_faulty: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildFlagRequest {
    pub is_faulty: bool,
}

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionListQuery {
    pub sort_by: Option<String>,
    pub desc: Option<String>,
    pub search: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionList {
    pub data: Vec<serde_json::Value>,
    pub total: u64,
    pub current_page: u64,
    pub total_pages: u64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActiveExecutionRecord {
    pub device_id: Option<String>,
    pub status: String,
    pub test_id: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCaseRecord {
    pub test_case_id: i64,
    pub suite_id: i64,
    pub result: Option<String>,
    pub comment: Option<String>,
    pub jira_defect: Option<String>,
    pub title: String,
    pub suite_name: String,
    pub script_file: Option<String>,
    pub id: i64,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
    pub output_file_path: Option<String>,
    pub dmesg_file_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReportRecord {
    pub id: Uuid,
    pub test_execution_id: String,
    pub status: String,
    pub device_type: String,
    pub build_version: String,
    pub created_by: String,
    pub upload_error: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BuildExecutionList {
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
    pub executions: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct EmptyObjectResponse {}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCaseInput {
    pub test_case_id: i64,
    pub name: Option<String>,
    pub execution_id: Option<String>,
    pub suite_id: i64,
    pub script_file: String,
    pub suite_name: String,
    pub title: String,
    pub plan_id: i64,
    pub order: Option<i64>,
    pub priority_id: Option<i64>,
    pub result: Option<String>,
    pub pre_condition: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSelectionExclusions {
    #[serde(default)]
    pub suites: Vec<String>,
    #[serde(default, alias = "cases")]
    pub cases_by_suite: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSuiteSelection {
    pub suite_id: String,
    pub select_all: bool,
    pub cases: Option<Vec<String>>,
    pub exclude_cases: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(tag = "mode")]
pub enum ExecutionSelection {
    #[serde(rename = "ALL")]
    All {
        #[serde(rename = "planId")]
        plan_id: String,
        #[serde(rename = "planName")]
        plan_name: Option<String>,
        exclude: Option<ExecutionSelectionExclusions>,
    },
    #[serde(rename = "PARTIAL")]
    Partial {
        #[serde(rename = "planId")]
        plan_id: String,
        #[serde(rename = "planName")]
        plan_name: Option<String>,
        suites: Vec<ExecutionSuiteSelection>,
    },
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateExecutionRequest {
    pub name: Option<String>,
    pub device_family: String,
    pub test_plan_name: Option<String>,
    pub device_type: String,
    pub device_id: Option<String>,
    pub build_id: String,
    pub test_queue_id: Option<String>,
    pub test_plan_id: Option<i64>,
    #[serde(default)]
    pub test_suites: Vec<i64>,
    #[serde(default)]
    pub is_all_selected: bool,
    pub logs: Option<Vec<String>>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    #[serde(default)]
    pub test_cases: BTreeMap<i64, Vec<ExecutionCaseInput>>,
    pub selection: Option<ExecutionSelection>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateExecutionRequest {
    #[serde(rename = "testID")]
    pub test_id: String,
    #[serde(default)]
    pub comments: String,
    pub build_id: Option<String>,
    #[serde(default)]
    pub log: bool,
    pub result: String,
}

fn default_log_type() -> String {
    "general".to_owned()
}
fn default_log_level() -> String {
    "info".to_owned()
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogCreateRequest {
    #[serde(rename = "type", default = "default_log_type")]
    pub log_type: String,
    pub reference_id: String,
    pub data: serde_json::Value,
    #[serde(default = "default_log_level")]
    pub level: String,
    #[schema(value_type = String, format = DateTime)]
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogListQuery {
    #[serde(rename = "type")]
    pub log_type: Option<String>,
    pub reference_id: Option<String>,
    pub level: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub query: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: i64,
    #[serde(rename = "type")]
    pub log_type: String,
    pub reference_id: String,
    pub data: serde_json::Value,
    pub level: String,
    #[schema(value_type = String, format = DateTime)]
    pub timestamp: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogList {
    pub logs: Vec<LogEntry>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTypeFolder {
    pub device_type: String,
    pub folder_name: String,
    pub device_family: String,
    pub default_version: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum DeviceTypeFolderRequest {
    One(DeviceTypeFolder),
    Many(Vec<DeviceTypeFolder>),
}

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
pub struct DefaultArtifactCopyQuery {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DefaultArtifactCopyResponse {
    pub success: bool,
    pub message: &'static str,
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
#[serde(rename_all = "camelCase")]
pub struct PaginationResponse {
    pub page: u32,
    pub limit: u32,
    pub total_count: i64,
    pub total_pages: u64,
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

#[derive(Debug, Serialize, ToSchema)]
pub struct TestCompletionResponse {
    #[schema(example = true)]
    pub success: bool,
    #[schema(example = "Test completion handled for device aabbccddeeff")]
    pub message: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceFlashingResponse {
    #[schema(example = true)]
    pub success: bool,
    #[schema(example = "Device aabbccddeeff is now marked as upgrading")]
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayChannelConfiguration {
    pub relay_id: Uuid,
    pub channel_id: Uuid,
    pub device_id: Option<String>,
    pub gpio: Option<String>,
    #[serde(default = "default_low_voltage")]
    pub gpio_default_level: VoltageLevel,
    #[serde(default = "default_low_voltage")]
    pub relay_default_level: VoltageLevel,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum VoltageLevel {
    Low,
    High,
}

fn default_low_voltage() -> VoltageLevel {
    VoltageLevel::Low
}

#[cfg(test)]
mod voltage_default_tests {
    use super::{RelayChannelConfiguration, VoltageLevel};

    #[test]
    fn relay_channel_voltage_levels_default_low() {
        let configuration: RelayChannelConfiguration = serde_json::from_value(serde_json::json!({
            "relayId": uuid::Uuid::new_v4(),
            "channelId": uuid::Uuid::new_v4(),
            "deviceId": "device-1",
            "gpio": "17"
        }))
        .unwrap();

        assert_eq!(configuration.gpio_default_level, VoltageLevel::Low);
        assert_eq!(configuration.relay_default_level, VoltageLevel::Low);
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum RelayConfigurationRequest {
    One(RelayChannelConfiguration),
    Many(Vec<RelayChannelConfiguration>),
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

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct AdminAlertListQuery {
    pub sort_by: Option<String>,
    pub desc: Option<String>,
    pub page: Option<String>,
    pub limit: Option<String>,
    pub search: Option<String>,
    pub filter: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct AllAlertListQuery {
    pub page: Option<String>,
    pub limit: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AlertList {
    pub data: Vec<serde_json::Value>,
    pub total_data: i64,
    pub total_pages: i64,
    pub current_page: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_unread_count: Option<i64>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum CompatibilityErrorResponse {
    Structured(ErrorResponse),
    Message(MessageResponse),
}

#[derive(Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportCreateRequest {
    pub device_type: String,
    pub device_family: String,
    pub release_id: Uuid,
    pub description: String,
    pub test_execution_id: Option<String>,
    pub attach_logs: Option<bool>,
    #[schema(format = Binary)]
    pub image: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum FaultyReportStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportRecord {
    pub id: Uuid,
    pub release_id: Uuid,
    pub device_type: String,
    pub device_family: String,
    pub last_test_execution_id: Option<String>,
    pub description: String,
    pub file_path: Option<String>,
    pub logs_path: Option<String>,
    pub status: FaultyReportStatus,
    pub created_by: String,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportUser {
    pub id: String,
    pub user_name: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportDetail {
    pub report: FaultyReportRecord,
    pub build_version: String,
    pub user: FaultyReportUser,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SuccessResponse {
    pub success: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FaultyReportStatusRequest {
    pub status: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RelayConfigurationConfirmation {
    #[schema(example = "aabbccddeeff")]
    pub mac: String,
    pub status: CallbackStatus,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelayConfigurationAccepted {
    pub message: &'static str,
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelayConfirmationResponse {
    pub confirmed: bool,
    pub message: String,
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) controller_id: Option<String>,
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) relay_id: Option<Uuid>,
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) device_id: Option<String>,
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) configuration_complete: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CallbackResponse {
    pub success: bool,
    pub message: &'static str,
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
