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
    #[serde(rename = "heartbeat")]
    HeartbeatData {
        device_id: Option<String>,
        controller_id: Option<String>,
        device_controller_id: Option<String>,
        timestamp: Option<String>,
        status: Option<String>,
        extra: Option<serde_json::Value>,
    },

    #[serde(rename = "alert")]
    AlertDataPayload {
        level: Option<AlertLevel>,
        subtype: Option<String>,
        message: serde_json::Value,
        timestamp: Option<String>,
    },

    #[serde(rename = "user_data")]
    UserDataPayload {
        method: String,
        message: serde_json::Value,
        timestamp: Option<String>,
    },

    #[serde(rename = "ssh_input")]
    SshInputPayload {
        // stream: StreamKind,
        chunk: String,
    },

    #[serde(rename = "ssh_output")]
    SshOutputPayload { stream: StreamKind, chunk: String },

    #[serde(rename = "device_state_update")]
    DeviceStateUpdate {
        device_id: String,
        state: DeviceState,
        upgrading: bool,
        flashing: bool,
        changed_at: String,
    },

    #[serde(rename = "device_power_update")]
    DevicePowerUpdate {
        device_id: String,
        power: DevicePower,
        changed_at: String,
    },

    #[serde(rename = "device_controller_changed")]
    DeviceControllerChanged {
        action: Action,
        controller_id: String,
    },

    #[serde(rename = "test_execution")]
    TestExecution {
        test_id: String,
        r#type: TestExecutionUpdateType,
        data: TestExecutionUpdateData,
    },

    #[serde(rename = "build_test_execution_update")]
    BuildTestExecutionUpdate {
        test_id: String,
        build_id: String,
        test_case_id: Option<String>,
        status: Option<String>,
    },

    #[serde(rename = "test_execution_update")]
    TestExecutionUpdate {
        build_id: Option<String>,
        test_id: String,
        status: Option<TestStatus>,
        progress: Option<u64>,
        message: Option<String>,
        cancel_requested: Option<bool>,
    },

    #[serde(rename = "build_performance_update")]
    BuildPerformanceUpdate { build_id: String },

    #[serde(rename = "build_status_changed")]
    BuildStatusChanged {
        release_id: Uuid,
        status: String,
        is_faulty: bool,
        device_id: Option<String>,
        device_type: String,
        version: Option<String>,
        user_id: String,
    },

    #[serde(rename = "build_uploaded")]
    BuildUploaded {
        release_id: String,
        version: String,
        device_type: String,
        device_family: String,
        build_type: String,
    },

    #[serde(rename = "build_deleted")]
    BuildDeleted {
        release_id: String,
        version: String,
        device_type: String,
        user_id: String,
    },

    #[serde(rename = "build_flagged")]
    BuildFlagged {
        release_id: String,
        is_faulty: bool,
        version: String,
        device_type: String,
        user_id: String,
    },

    #[serde(rename = "upload_progress")]
    UploadProgress { upload_id: String, percent: u64 },

    #[serde(rename = "upload_error")]
    UploadError { upload_id: String, message: String },

    #[serde(rename = "upload_complete")]
    UploadComplete {
        upload_id: String,
        file_name: String,
        tag: Option<String>,
    },

    #[serde(rename = "relay_configuration_status")]
    RelayConfigurationStatus {
        relay_id: Option<uuid::Uuid>,
        controller_id: Option<String>,
        status: String,
        total: usize,
        completed: usize,
        errors: Vec<String>,
        message: Option<String>,
    },

    #[serde(rename = "rtos_log")]
    RtosLogPayload {
        test_id: String,
        build_id: Option<String>,
        path: String,
        user_id: String,
    },

    #[serde(rename = "device_controller_ping")]
    DeviceControllerPing {
        controller_id: String,
        timestamp: String,
        metrics: Metrics,
    },
    #[serde(rename = "device_ping")]
    DevicePingPayload {
        device_id: String,
        timestamp: String,
    },

    #[serde(rename = "device_interface_update")]
    DeviceInterfaceUpdate {
        device_id: String,
        changes: Vec<InterfaceChange>,
        heartbeat_data: HeartbeatData,
        timestamp: String,
    },

    #[serde(rename = "execution_report_update")]
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
#[serde(rename_all = "snake_case")]
pub enum TestExecutionUpdateType {
    Status,
    Log,
    New,
    CancelRequested,
    RtosLog,
    Queued,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "data", rename_all = "snake_case")]
pub struct TestExecutionUpdateData {
    pub status: Option<TestStatus>,
    pub message: Option<String>,
    pub timestamp: String,
    pub cancel_requested: Option<bool>,
    pub phase: Option<String>,
    pub rtos_log_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
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
#[serde(rename_all = "snake_case")]
pub enum DevicePower {
    On,
    Off,
}

impl From<&str> for DevicePower {
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
