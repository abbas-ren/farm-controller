//! Shared DTOs returned by device repository / store operations.
//!
//! These structs carry the side-effect payloads handlers and workers need
//! after persistence (alerts, edge targets, deferred cancellations). They
//! replace the ad-hoc objects returned by Node `DeviceService` / `TestService`
//! / `RelayService` method results.

use std::path::PathBuf;
use uuid::Uuid;

use crate::socket::types::farm::ServerEvent;

use super::{DeviceController, ExecutionCaseInput, ExecutionSelection};

/// Input for creating a pending faulty report before file upload.
#[derive(Debug)]
pub(crate) struct FaultyReportCreate {
    /// Pre-allocated report UUID (matches upload path on disk).
    pub id: Uuid,
    /// Release UUID string being reported faulty.
    pub release_id: String,
    pub device_type: String,
    pub device_family: String,
    pub description: String,
    pub created_by: String,
    /// Optional explicit test id; otherwise store resolves the latest.
    pub test_execution_id: Option<String>,
}

/// Result of inserting a faulty report (resolved last execution id).
#[derive(Debug)]
pub(crate) struct FaultyReportCreation {
    pub last_test_execution_id: Option<String>,
}

/// Report + admin alert JSON after upload finalization.
#[derive(Debug)]
pub(crate) struct FaultyReportFinalization {
    /// Faulty report API JSON for the response body.
    pub report: serde_json::Value,
    /// Alert payload to publish on the frontend event hub.
    pub alert: serde_json::Value,
}

/// Device upsert result plus which socket notifications to emit.
#[derive(Debug)]
pub(crate) struct DeviceRegistrationResult {
    pub device: serde_json::Value,
    /// True when a new device row was created (addition toast).
    pub notify_addition: bool,
    /// True when status became approved (approval toast).
    pub notify_approval: bool,
}

/// Fully resolved create-execution request ready for persistence.
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

/// Outcome of requesting test cancellation (for edge cleanup + UI events).
#[derive(Clone, Debug)]
pub(crate) struct TestCancellationResult {
    pub build_id: String,
    pub created_by: String,
    /// Execution phase string at cancel time (queued/in_progress/…).
    pub phase: String,
    /// Whether status actually changed.
    pub changed: bool,
    /// Whether the execution reached a terminal cancelled state.
    pub terminalized: bool,
    /// Assigned DUT freed for reassignment (when terminalized with a device).
    pub device_id: Option<String>,
    pub device_ip: Option<String>,
    /// Device type for NFS `currentTest.txt` cleanup (even when cancel is deferred).
    pub device_type: Option<String>,
    /// Registered device NFS root (Mode A currentTest lives here, not only under `{type}/{testId}`).
    pub device_nfs_path: Option<String>,
    pub cleanup_device_type: Option<String>,
    pub test_cycle_id: Option<String>,
}

/// Edge teardown targets gathered when a test completes.
#[derive(Clone, Debug)]
pub(crate) struct TestCompletionTarget {
    pub status: String,
    pub device_family: Option<String>,
    pub mac_address: Option<String>,
    pub device_type: String,
    pub build_version: Option<String>,
    pub build_id: Option<String>,
    pub created_by: Option<String>,
    /// Gen5 controller IP for UART/RTOS cleanup.
    pub gen5_controller_ip: Option<String>,
    pub uart_port: Option<String>,
    /// Gen4 controller IP for relay cleanup.
    pub gen4_controller_ip: Option<String>,
    pub relay_serial: Option<String>,
    pub relay_channel: Option<i32>,
}

/// Where to capture Gen5 RTOS logs after flashing.
#[derive(Clone, Debug)]
pub(crate) struct RtosCaptureTarget {
    pub controller_ip: String,
    pub mac_address: String,
    pub generation: u8,
    pub rtos_port: Option<String>,
    pub relay_serial: Option<String>,
    pub relay_channel: Option<i32>,
}

/// Device JSON plus optional RTOS capture target after mark-flashing.
#[derive(Clone, Debug)]
pub(crate) struct DeviceFlashingResult {
    pub device: serde_json::Value,
    pub rtos_target: Option<RtosCaptureTarget>,
}

/// Paths and metadata needed to finalize a build upload into a release row.
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
    pub rootfs_archive_extension: String,
}

/// Release + alert JSON after a successful upload finalize.
#[derive(Clone, Debug)]
pub(crate) struct BuildUploadResult {
    pub release: serde_json::Value,
    pub alert: serde_json::Value,
}

/// Filesystem cleanup target and alert after deleting a build.
#[derive(Clone, Debug)]
pub(crate) struct BuildDeleteTarget {
    pub folder_name: String,
    pub version: String,
    pub device_family: String,
    pub device_type: String,
    pub alert: serde_json::Value,
}

/// Alert payload after flagging a build.
#[derive(Clone, Debug)]
pub(crate) struct BuildFlagResult {
    pub version: String,
    pub device_type: String,
    pub alert: serde_json::Value,
}

/// Alert payload after updating build status (e.g. faulty).
#[derive(Clone, Debug)]
pub(crate) struct BuildStatusResult {
    pub version: String,
    pub device_type: String,
    pub is_faulty: bool,
    pub alert: serde_json::Value,
}

/// One CSV export row for a user's test execution.
#[derive(Clone, Debug)]
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

/// Edge address needed before deleting a controller.
#[derive(Clone, Debug)]
pub(crate) struct ControllerDeleteTarget {
    pub ip_address: String,
    pub device_family: Option<String>,
}

/// Edge/relay targets needed before deleting a device.
#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct DeviceDeleteTarget {
    pub ip_address: String,
    pub device_family: Option<String>,
    pub controller_id: Option<String>,
    pub controller_ip: Option<String>,
    pub relay_serial: Option<String>,
    pub relay_channel: Option<i32>,
}

/// Controller power-port target for a reboot command.
#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct DeviceRebootTarget {
    pub device_family: Option<String>,
    pub controller_ip: Option<String>,
    pub power_port: Option<String>,
}

/// Power on/off target (Gen5 mapping port or Gen4 relay channel).
#[derive(Clone, Debug, sqlx::FromRow, sea_orm::FromQueryResult)]
pub(crate) struct DevicePowerTarget {
    pub device_family: Option<String>,
    pub current_power: Option<String>,
    pub controller_ip: Option<String>,
    pub power_port: Option<String>,
    pub relay_serial: Option<String>,
    pub channel_id: Option<Uuid>,
    pub channel_number: Option<i32>,
}

/// Relay channel identity for toggling channel state on the edge.
#[derive(Clone, Debug, sqlx::FromRow, sea_orm::FromQueryResult)]
pub(crate) struct RelayStateTarget {
    pub controller_ip: String,
    pub relay_serial: String,
    pub channel_id: Uuid,
    pub channel_number: i32,
}

/// Controller + current serial for a relay identity remap.
#[derive(Clone, Debug, sqlx::FromRow, sea_orm::FromQueryResult)]
pub(crate) struct RelayIdentityTarget {
    pub controller_id: String,
    pub controller_ip: String,
    pub current_serial: String,
}

/// Approved device address for outbound device-action HTTP.
#[derive(Clone, Debug)]
pub(crate) struct DeviceActionTarget {
    pub device_id: String,
    pub ip_address: String,
    pub status: String,
}

/// Controller registration upsert result.
#[derive(Clone, Debug)]
pub(crate) struct RegistrationResult {
    /// True when a new controller row was inserted.
    pub created: bool,
    pub controller: DeviceController,
}

/// Controller heartbeat persistence outcome.
#[derive(Clone, Debug)]
pub(crate) struct ControllerHeartbeatResult {
    pub state_changed: bool,
    pub alert: Option<serde_json::Value>,
}

/// Device heartbeat persistence outcome including deferred worker work.
#[derive(Clone, Debug)]
pub(crate) struct DeviceHeartbeatResult {
    pub state: String,
    pub state_changed: bool,
    pub interface_changes: Vec<serde_json::Value>,
    pub heartbeat_data: serde_json::Value,
    pub alerts: Vec<serde_json::Value>,
    #[allow(unused)]
    pub post_update_events: Vec<ServerEvent>,
    /// Cancellations to run after the heartbeat transaction commits.
    pub deferred_cancellations: Vec<DeferredCancellation>,
    /// Test log lines to append after commit.
    pub deferred_test_logs: Vec<DeferredTestLog>,
}

/// Cancellation to enqueue after a heartbeat frees a busy device mid-test.
#[derive(Clone, Debug)]
pub(crate) struct DeferredCancellation {
    pub test_id: String,
    pub build_id: String,
    #[allow(unused)]
    pub created_by: String,
}

/// Progress line to append to a test log after heartbeat commit.
#[derive(Clone, Debug)]
pub(crate) struct DeferredTestLog {
    pub test_id: String,
    pub message: String,
}

/// Flash-confirmation side effects for UI progress / cancel / free-device.
#[derive(Clone, Debug, Default)]
pub(crate) struct FlashConfirmationResult {
    pub progress: Option<FlashExecutionProgress>,
    pub cancelled_execution: Option<FlashCancelledExecution>,
    pub freed_device_id: Option<String>,
}

/// Execution cancelled because flash failed or was aborted.
#[derive(Clone, Debug)]
pub(crate) struct FlashCancelledExecution {
    pub test_id: String,
    pub build_id: String,
    #[allow(unused)]
    pub created_by: String,
}

/// Flash progress event for the frontend during IPL confirmation.
#[derive(Clone, Debug)]
pub(crate) struct FlashExecutionProgress {
    pub test_id: String,
    pub build_id: String,
    pub completed: bool,
    pub percentage: u8,
    pub message: String,
    #[allow(unused)]
    pub created_by: String,
}

/// Optional structured heartbeat event (reserved for socket fan-out).
#[derive(Clone, Debug)]
#[allow(unused)]
pub(crate) struct DeviceHeartbeatEvent {
    pub event: String,
    pub payload: serde_json::Value,
}

/// Outbound edge action required after relay channel configuration changes.
#[derive(Clone, Debug)]
pub enum RelayControllerAction {
    /// Clear a prior device ↔ channel mapping on the controller.
    Remove {
        controller_address: String,
        device_mac: String,
        relay_serial: String,
        channel_number: i32,
    },
    /// Push a new device ↔ channel mapping to the controller.
    Configure {
        controller_address: String,
        device_mac: String,
        relay_serial: String,
        channel_number: i32,
        device_generation: Option<String>,
    },
}

/// Updated channel JSON plus edge actions after configure/remap.
#[derive(Clone, Debug)]
pub(crate) struct RelayConfigurationResult {
    pub channels: Vec<serde_json::Value>,
    pub actions: Vec<RelayControllerAction>,
}
