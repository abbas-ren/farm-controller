use async_trait::async_trait;

use super::*;

/// Owns device and controller heartbeat persistence.
#[async_trait]
pub(crate) trait HeartbeatRepository: Send + Sync {
    async fn record_controller_heartbeat(
        &self,
        heartbeat: &ControllerHeartbeat,
    ) -> Result<ControllerHeartbeatResult, DeviceRepositoryError>;
    async fn record_device_heartbeat(
        &self,
        device_id: &str,
        heartbeat: &serde_json::Value,
    ) -> Result<Option<DeviceHeartbeatResult>, DeviceRepositoryError>;
}
