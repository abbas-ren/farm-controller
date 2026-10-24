//! Shared socket route paths and room/event name constants.
//!
//! Event names match live `FrontendMessagePayload` serde `kind` tags (camelCase).

pub const EDGE_SUBPROTOCOL: &str = "web-cli-protocol";
pub const BROWSER_SUBPROTOCOL: &str = "farm-browser-v1";
pub const BROWSER_BEARER_PREFIX: &str = "bearer.";
pub const WS_EVENT: WsEvent = WsEvent {
    ALERT: "alert",

    HEARTBEAT: "heartbeat",

    USER_DATA: "userData",

    SSH_INPUT: "sshInput",
    SSH_OUTPUT: "sshOutput",

    DEVICE_INTERFACE_UPDATE: "deviceInterfaceUpdate",
    DEVICE_PING: "devicePing",
    DEVICE_POWER_UPDATE: "devicePowerUpdate",
    DEVICE_STATE_UPDATE: "deviceStateUpdate",

    DEVICE_CONTROLLER_CHANGED: "deviceControllerChanged",
    DEVICE_CONTROLLER_PING: "deviceControllerPing",

    TEST_EXECUTION: "testExecution",
    TEST_EXECUTION_UPDATE: "testExecutionUpdate",

    BUILD_DELETED: "buildDeleted",
    BUILD_PERFORMANCE_UPDATE: "buildPerformanceUpdate",
    BUILD_STATUS_CHANGED: "buildStatusChanged",
    BUILD_TEST_EXECUTION_UPDATE: "buildTestExecutionUpdate",
    BUILD_UPLOADED: "buildUploaded",

    UPLOAD_PROGRESS: "uploadProgress",
    UPLOAD_ERROR: "uploadError",
    UPLOAD_COMPLETE: "uploadComplete",

    RELAY_CONFIG_STATUS: "relayConfigurationStatus",

    RTS_LOG: "rtosLog",
};

#[allow(non_snake_case)]
pub struct WsEvent {
    pub ALERT: &'static str,
    pub HEARTBEAT: &'static str,
    pub USER_DATA: &'static str,
    pub SSH_INPUT: &'static str,
    pub SSH_OUTPUT: &'static str,
    pub DEVICE_INTERFACE_UPDATE: &'static str,
    pub DEVICE_PING: &'static str,
    pub DEVICE_POWER_UPDATE: &'static str,
    pub DEVICE_STATE_UPDATE: &'static str,
    pub DEVICE_CONTROLLER_CHANGED: &'static str,
    pub DEVICE_CONTROLLER_PING: &'static str,
    pub TEST_EXECUTION: &'static str,
    pub TEST_EXECUTION_UPDATE: &'static str,
    pub BUILD_DELETED: &'static str,
    pub BUILD_PERFORMANCE_UPDATE: &'static str,
    pub BUILD_STATUS_CHANGED: &'static str,
    pub BUILD_TEST_EXECUTION_UPDATE: &'static str,
    pub BUILD_UPLOADED: &'static str,
    pub UPLOAD_PROGRESS: &'static str,
    pub UPLOAD_ERROR: &'static str,
    pub UPLOAD_COMPLETE: &'static str,
    pub RELAY_CONFIG_STATUS: &'static str,
    pub RTS_LOG: &'static str,
}
