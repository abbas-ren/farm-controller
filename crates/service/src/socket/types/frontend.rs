//! Frontend-facing message payloads and listener contract types.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{reports::ReportStatus, workers::types::DeviceState};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamKind {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    #[serde(rename = "added")]
    Added,
    #[serde(rename = "updated")]
    Updated,
    #[serde(rename = "deleted")]
    Deleted,
}

impl From<&str> for Action {
    /// From.
    fn from(action: &str) -> Self {
        match action {
            "added" => Action::Added,
            "updated" => Action::Updated,
            "deleted" => Action::Deleted,
            _ => Action::Updated, // default case
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendMessagePayload {
    #[serde(rename = "heartbeat", rename_all = "camelCase")]
    HeartbeatData {
        device_id: Option<String>,
        controller_id: Option<String>,
        device_controller_id: Option<String>,
        timestamp: Option<String>,
        status: Option<String>,
        extra: Option<serde_json::Value>,
    },

    #[serde(rename = "alert", rename_all = "camelCase")]
    AlertDataPayload {
        level: Option<AlertLevel>,
        subtype: Option<String>,
        message: serde_json::Value,
        timestamp: Option<String>,
    },

    #[serde(rename = "userData", rename_all = "camelCase")]
    UserDataPayload {
        method: String,
        message: serde_json::Value,
        timestamp: Option<String>,
    },

    #[serde(rename = "sshInput", rename_all = "camelCase")]
    SshInputPayload {
        // stream: StreamKind,
        chunk: String,
    },

    #[serde(rename = "sshOutput", rename_all = "camelCase")]
    SshOutputPayload { stream: StreamKind, chunk: String },

    #[serde(rename = "deviceStateUpdate", rename_all = "camelCase")]
    DeviceStateUpdate {
        device_id: String,
        state: DeviceState,
        upgrading: bool,
        flashing: bool,
        changed_at: String,
    },

    #[serde(rename = "devicePowerUpdate", rename_all = "camelCase")]
    DevicePowerUpdate {
        device_id: String,
        power: DevicePower,
        changed_at: String,
    },

    #[serde(rename = "deviceControllerChanged", rename_all = "camelCase")]
    DeviceControllerChanged {
        action: Action,
        controller_id: String,
    },

    #[serde(rename = "testExecution", rename_all = "camelCase")]
    TestExecution {
        test_id: String,
        r#type: TestExecutionUpdateType,
        data: TestExecutionUpdateData,
    },

    #[serde(rename = "buildTestExecutionUpdate", rename_all = "camelCase")]
    BuildTestExecutionUpdate {
        test_id: String,
        build_id: String,
        test_case_id: Option<String>,
        status: Option<String>,
    },

    #[serde(rename = "testExecutionUpdate", rename_all = "camelCase")]
    TestExecutionUpdate {
        build_id: Option<String>,
        test_id: String,
        status: Option<TestStatus>,
        progress: Option<u64>,
        message: Option<String>,
        cancel_requested: Option<bool>,
    },

    #[serde(rename = "buildPerformanceUpdate", rename_all = "camelCase")]
    BuildPerformanceUpdate { build_id: String },

    #[serde(rename = "buildStatusChanged", rename_all = "camelCase")]
    BuildStatusChanged {
        release_id: Uuid,
        status: String,
        is_faulty: bool,
        device_id: Option<String>,
        device_type: String,
        version: Option<String>,
        user_id: String,
    },

    #[serde(rename = "buildUploaded", rename_all = "camelCase")]
    BuildUploaded {
        release_id: String,
        version: String,
        device_type: String,
        device_family: String,
        build_type: String,
    },

    #[serde(rename = "buildDeleted", rename_all = "camelCase")]
    BuildDeleted {
        release_id: String,
        version: String,
        device_type: String,
        user_id: String,
    },

    #[serde(rename = "buildFlagged", rename_all = "camelCase")]
    BuildFlagged {
        release_id: String,
        is_faulty: bool,
        version: String,
        device_type: String,
        user_id: String,
    },

    #[serde(rename = "uploadProgress", rename_all = "camelCase")]
    UploadProgress { upload_id: String, percent: u64 },

    #[serde(rename = "uploadError", rename_all = "camelCase")]
    UploadError { upload_id: String, message: String },

    #[serde(rename = "uploadComplete", rename_all = "camelCase")]
    UploadComplete {
        upload_id: String,
        file_name: String,
        tag: Option<String>,
    },

    #[serde(rename = "relayConfigurationStatus", rename_all = "camelCase")]
    RelayConfigurationStatus {
        relay_id: Option<uuid::Uuid>,
        controller_id: Option<String>,
        status: String,
        total: usize,
        completed: usize,
        errors: Vec<String>,
        message: Option<String>,
    },

    #[serde(rename = "rtosLog", rename_all = "camelCase")]
    RtosLogPayload {
        test_id: String,
        build_id: Option<String>,
        path: String,
        user_id: String,
    },

    #[serde(rename = "deviceControllerPing", rename_all = "camelCase")]
    DeviceControllerPing {
        controller_id: String,
        timestamp: String,
        metrics: Metrics,
    },
    #[serde(rename = "devicePing", rename_all = "camelCase")]
    DevicePingPayload {
        device_id: String,
        timestamp: String,
    },

    #[serde(rename = "deviceInterfaceUpdate", rename_all = "camelCase")]
    DeviceInterfaceUpdate {
        device_id: String,
        changes: Vec<InterfaceChange>,
        heartbeat_data: HeartbeatData,
        timestamp: String,
    },

    #[serde(rename = "executionReportUpdate", rename_all = "camelCase")]
    ExecutionReportUpdate {
        test_execution_id: Option<String>,
        status: Option<ReportStatus>,
        error: Option<String>,
        upload_error: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceChange {
    pub r#type: String,
    pub interface_id: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub network: NetworkMetrics,
    pub disk: DiskMetrics,
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuMetrics {
    pub current: String,
    pub total: String,
    pub usage_percent: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetrics {
    pub used: String,
    pub total: String,
    pub usage_percent: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkMetrics {
    pub upload: String,
    pub download: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskMetrics {
    pub used: String,
    pub total: String,
    pub usage_percent: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TestExecutionUpdateType {
    Status,
    Log,
    New,
    CancelRequested,
    RtosLog,
    Queued,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "data", rename_all = "camelCase")]
pub struct TestExecutionUpdateData {
    pub status: Option<TestStatus>,
    pub message: Option<String>,
    pub timestamp: String,
    pub cancel_requested: Option<bool>,
    pub phase: Option<String>,
    pub rtos_log_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TestStatus {
    NotExecuted,
    InProgress,
    Completed,
    Queued,
    Cancelled,
    CancelRequested,
    Failed,
}

impl From<&str> for TestStatus {
    /// From.
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "not_executed" => Self::NotExecuted,
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "queued" => Self::Queued,
            "cancelled" => Self::Cancelled,
            "cancel_requested" => Self::CancelRequested,
            "failed" => Self::Failed,
            _ => Self::NotExecuted,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DevicePower {
    On,
    Off,
}

impl From<&str> for DevicePower {
    /// From.
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "on" => Self::On,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestStatusData {
    pub status: String,
    pub timestamp: Option<String>,
    pub progress: Option<f64>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestcaseData {
    pub test_case_id: String,
    pub result: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedData {
    pub status: String,
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelRequestedData {
    pub cancel_requested: bool,
    pub phase: String,
    pub timestamp: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatData {
    pub device_id: Option<String>,
    pub controller_id: Option<String>,
    pub device_controller_id: Option<String>,
    pub timestamp: Option<String>,
    pub status: Option<String>,
    pub extra: Option<serde_json::Value>,
}
