use serde::Serialize;

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

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendMessagePayload {
    #[serde(rename = "heartbeat")]
    HeartbeatDataPayload {
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

    #[serde(rename = "user")]
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
        state: String,
        upgrading: bool,
        flashing: bool,
        changed_at: String,
    },

    #[serde(rename = "device_power_update")]
    DevicePowerUpdate {
        device_id: String,
        power: String,
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
        build_id: Option<String>,
        created_by: Option<String>,
        update: TestExecutionUpdate,
    },

    #[serde(rename = "build_test_execution_update")]
    BuildTestExecutionUpdate {
        test_id: String,
        build_id: String,
        test_case_id: Option<String>,
    },

    #[serde(rename = "test_execution_update")]
    TestExecutionUpdate {
        build_id: String,
        test_id: String,
        status: String,
        progress: Option<u64>,
        message: Option<String>,
        cancel_requested: Option<bool>,
    },

    #[serde(rename = "build_performance_update")]
    BuildPerformanceUpdate { build_id: String },

    #[serde(rename = "build_status_changed")]
    BuildStatusChanged {
        release_id: uuid::Uuid,
        status: String,
        is_faulty: bool,
        device_id: Option<String>,
        device_type: String,
        version: Option<String>,
        user_id: String,
    },

    #[serde(rename = "build_uploaded")]
    BuildUploaded {
        release_id: serde_json::Value,
        version: serde_json::Value,
        device_type: serde_json::Value,
        device_family: serde_json::Value,
        build_type: serde_json::Value,
    },

    #[serde(rename = "build_deleted")]
    BuildDeleted { release_id: String },

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

    #[serde(rename = "edge_controller_ping")]
    EdgeControllerPing {
        controller_id: String,
        timestamp: String,
        metrics: Metrics,
    },
    #[serde(rename = "edge_ping")]
    EdgePingPayload {
        device_id: String,
        timestamp: String,
    },

    #[serde(rename = "edge_interface_update")]
    InterfaceUpdatePayload {
        device_id: String,
        changes: Vec<InterfaceChange>,
        heartbeat_data: serde_json::Value,
        timestamp: String,
    },
}

#[derive(Clone, Debug, Serialize)]
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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TestExecutionUpdate {
    Status { data: TestStatusData },

    Testcase { data: TestcaseData },

    Queued { data: QueuedData },

    CancelRequested { data: CancelRequestedData },
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
