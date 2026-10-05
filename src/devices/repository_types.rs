use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug)]
pub(crate) struct FaultyReportCreate {
    pub id: Uuid,
    pub release_id: String,
    pub device_type: String,
    pub device_family: String,
    pub description: String,
    pub created_by: String,
    pub test_execution_id: Option<String>,
}

#[derive(Debug)]
pub(crate) struct FaultyReportCreation {
    pub last_test_execution_id: Option<String>,
}

#[derive(Debug)]
pub(crate) struct FaultyReportFinalization {
    pub report: serde_json::Value,
    pub alert: serde_json::Value,
}

use super::{DeviceController, ExecutionCaseInput, ExecutionSelection};

#[derive(Debug)]
pub(crate) struct DeviceRegistrationResult {
    pub device: serde_json::Value,
    pub notify_addition: bool,
    pub notify_approval: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ExecutionCreation {
    pub name: Option<String>,
    pub device_family: String,
    pub device_type: String,
    pub build_id: String,
    pub plan_id: i64,
    pub plan_name: String,
    pub test_suites: Vec<i64>,
    pub is_all_selected: bool,
    pub selection: Option<ExecutionSelection>,
    pub cases: Vec<ExecutionCaseInput>,
    pub user_id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct TestCancellationResult {
    pub build_id: String,
    pub created_by: String,
    pub phase: String,
    pub changed: bool,
    pub terminalized: bool,
    pub device_ip: Option<String>,
    pub cleanup_device_type: Option<String>,
    pub test_cycle_id: Option<String>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct TestCompletionTarget {
    pub status: String,
    pub device_family: Option<String>,
    pub mac_address: Option<String>,
    pub device_type: String,
    pub build_version: Option<String>,
    pub build_id: Option<String>,
    pub created_by: Option<String>,
    pub gen5_controller_ip: Option<String>,
    pub uart_port: Option<String>,
    pub gen4_controller_ip: Option<String>,
    pub relay_serial: Option<String>,
    pub relay_channel: Option<i32>,
}

#[derive(Clone, Debug)]
pub(crate) struct RtosCaptureTarget {
    pub controller_ip: String,
    pub mac_address: String,
    pub generation: u8,
    pub rtos_port: Option<String>,
    pub relay_serial: Option<String>,
    pub relay_channel: Option<i32>,
}

#[derive(Clone, Debug)]
pub(crate) struct DeviceFlashingResult {
    pub device: serde_json::Value,
    pub rtos_target: Option<RtosCaptureTarget>,
}

#[derive(Clone, Debug)]
pub(crate) struct BuildUploadFinalization {
    pub source_path: PathBuf,
    pub artifacts_root: PathBuf,
    pub upload_id: String,
    pub filename: String,
    pub tag: Option<String>,
    pub is_custom: bool,
    pub user_id: String,
    pub max_expanded_bytes: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct BuildUploadResult {
    pub release: serde_json::Value,
    pub alert: serde_json::Value,
}

#[derive(Clone, Debug)]
pub(crate) struct BuildDeleteTarget {
    pub folder_name: String,
    pub version: String,
    pub device_family: String,
    pub device_type: String,
    pub alert: serde_json::Value,
}

#[derive(Clone, Debug)]
pub(crate) struct BuildFlagResult {
    pub version: String,
    pub device_type: String,
    pub alert: serde_json::Value,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct TestExportRow {
    pub test_plan_name: String,
    pub status: String,
    pub device_type: Option<String>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    pub test_cycle_id: Option<String>,
    pub total: i64,
    pub passed: i64,
    pub failed: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct ControllerDeleteTarget {
    pub ip_address: String,
    pub device_family: Option<String>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct DeviceDeleteTarget {
    pub ip_address: String,
    pub device_family: Option<String>,
    pub controller_id: Option<String>,
    pub controller_ip: Option<String>,
    pub relay_serial: Option<String>,
    pub relay_channel: Option<i32>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct DeviceRebootTarget {
    pub device_family: Option<String>,
    pub controller_ip: Option<String>,
    pub power_port: Option<String>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct DevicePowerTarget {
    pub device_family: Option<String>,
    pub current_power: Option<String>,
    pub controller_ip: Option<String>,
    pub power_port: Option<String>,
    pub relay_serial: Option<String>,
    pub channel_id: Option<Uuid>,
    pub channel_number: Option<i32>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct RelayStateTarget {
    pub controller_ip: String,
    pub relay_serial: String,
    pub channel_id: Uuid,
    pub channel_number: i32,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct RelayIdentityTarget {
    pub controller_id: String,
    pub controller_ip: String,
    pub current_serial: String,
}

#[derive(Clone, Debug)]
pub(crate) struct DeviceActionTarget {
    pub device_id: String,
    pub ip_address: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RegistrationResult {
    pub created: bool,
    pub controller: DeviceController,
}

#[derive(Clone, Debug)]
pub(crate) struct ControllerHeartbeatResult {
    pub state_changed: bool,
    pub alert: Option<serde_json::Value>,
}

#[derive(Clone, Debug)]
pub(crate) struct DeviceHeartbeatResult {
    pub state: String,
    pub state_changed: bool,
    pub interface_changes: Vec<serde_json::Value>,
    pub heartbeat_data: serde_json::Value,
    pub alerts: Vec<serde_json::Value>,
    pub post_update_events: Vec<DeviceHeartbeatEvent>,
    pub deferred_cancellations: Vec<DeferredCancellation>,
}

#[derive(Clone, Debug)]
pub(crate) struct DeferredCancellation {
    pub test_id: String,
    pub build_id: String,
    pub created_by: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FlashConfirmationResult {
    pub cancelled_execution: Option<FlashCancelledExecution>,
    pub freed_device_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct FlashCancelledExecution {
    pub test_id: String,
    pub build_id: String,
    pub created_by: String,
}

#[derive(Clone, Debug)]
pub(crate) struct DeviceHeartbeatEvent {
    pub event: String,
    pub payload: serde_json::Value,
    pub room: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum RelayControllerAction {
    Remove {
        controller_address: String,
        device_mac: String,
        relay_serial: String,
        channel_number: i32,
    },
    Configure {
        controller_address: String,
        device_mac: String,
        relay_serial: String,
        channel_number: i32,
        device_generation: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct RelayConfigurationResult {
    pub channels: Vec<serde_json::Value>,
    pub actions: Vec<RelayControllerAction>,
}
