use async_trait::async_trait;

use super::*;

/// Owns device inventory, topology, and device deletion persistence.
#[async_trait]
pub(crate) trait DeviceInventoryRepository: Send + Sync {
    async fn device_families(&self) -> Result<Vec<String>, DeviceRepositoryError>;
    async fn device_types(
        &self,
        device_family: Option<&str>,
    ) -> Result<Vec<String>, DeviceRepositoryError>;
    async fn device_by_id(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn latest_heartbeat(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn topology(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn list_devices(
        &self,
        query: &DeviceListQuery,
        is_admin: bool,
    ) -> Result<DeviceList, DeviceRepositoryError>;
}
