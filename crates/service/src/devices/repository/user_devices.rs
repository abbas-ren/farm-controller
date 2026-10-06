use async_trait::async_trait;

use super::*;

/// Owns user-facing device listings, active device listings, and heartbeat timeout updates.
#[async_trait]
pub(crate) trait UserDeviceRepository: Send + Sync {
    async fn list_user_devices(
        &self,
        query: &UserDeviceListQuery,
    ) -> Result<DeviceList, DeviceRepositoryError>;
    async fn active_devices(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn update_heartbeat_timeout(
        &self,
        device_id: &str,
        value: i64,
    ) -> Result<bool, DeviceRepositoryError>;
}
