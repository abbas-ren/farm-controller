//! Shared socket route paths and room/event name constants.

pub const EDGE_SUBPROTOCOL: &str = "web-cli-protocol";
pub const BROWSER_SUBPROTOCOL: &str = "farm-browser-v1";
pub const BROWSER_BEARER_PREFIX: &str = "bearer.";
pub const WS_EVENT: WsEvent = WsEvent {
    ALERT: "alert",

    HEARTBEAT: "heartbeat",

    USER_DATA: "user_data",

    SSH_INPUT: "ssh_input",
    SSH_OUTPUT: "ssh_output",

    DEVICE_INTERFACE_UPDATE: "device_interface_update",
    DEVICE_PING: "device_ping",
    DEVICE_POWER_UPDATE: "device_power_update",
    DEVICE_STATE_UPDATE: "device_state_update",

    DEVICE_CONTROLLER_CHANGED: "device_controller_changed",
    DEVICE_CONTROLLER_PING: "device_controller_ping",

    TEST_EXECUTION: "test_execution",
    TEST_EXECUTION_UPDATE: "test_execution_update",

    BUILD_DELETED: "build_deleted",
    BUILD_PERFORMANCE_UPDATE: "build_performance_update",
    BUILD_STATUS_CHANGED: "build_status_changed",
    BUILD_TEST_EXECUTION_UPDATE: "build_test_execution_update",
    BUILD_UPLOADED: "build_uploaded",

    UPLOAD_PROGRESS: "upload_progress",
    UPLOAD_ERROR: "upload_error",
    UPLOAD_COMPLETE: "upload_complete",

    RELAY_CONFIG_STATUS: "relay_configuration_status",

    RTS_LOG: "rts_log",
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
