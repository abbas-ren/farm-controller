//! DTOs for the device heartbeat timeout worker (legacy `HeartbeatQueueService` timeouts).

use super::common::DeviceState;

/// Device whose last heartbeat exceeded the faulty / not-reachable threshold.
#[derive(Debug, sea_orm::FromQueryResult)]
pub struct StaleDevice {
    pub device_id: String,
    pub previous_state: String,
    pub target_state: String,
    /// Configured timeout (seconds) that triggered this transition.
    pub timeout: i32,
}

/// Published device state change plus optional alert JSON.
pub struct DeviceStateTransition {
    pub device_id: String,
    pub state: DeviceState,
    pub alert: serde_json::Value,
}
