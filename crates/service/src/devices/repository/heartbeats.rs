//! Device and controller heartbeat capability trait.
//!
//! Used by heartbeat workers (Node `HeartbeatQueueService`).

use async_trait::async_trait;

use super::*;

/// Owns device and controller heartbeat persistence.
#[async_trait]
pub(crate) trait HeartbeatRepository: Send + Sync {
    /// PURPOSE: Persist controller heartbeat and reachability alerts.
    /// USE: Controller heartbeat worker/handler.
    async fn record_controller_heartbeat(
        &self,
        heartbeat: &ControllerHeartbeat,
    ) -> Result<ControllerHeartbeatResult, DeviceRepositoryError>;
    /// PURPOSE: Persist device heartbeat, interfaces, and deferred work.
    /// USE: Device heartbeat worker/handler.
    async fn record_device_heartbeat(
        &self,
        device_id: &str,
        heartbeat: &serde_json::Value,
    ) -> Result<Option<DeviceHeartbeatResult>, DeviceRepositoryError>;
}
