//! Device inventory / topology capability trait.

use async_trait::async_trait;

use super::*;

/// Owns device inventory, topology, and device deletion persistence.
#[async_trait]
pub(crate) trait DeviceInventoryRepository: Send + Sync {
    /// PURPOSE: Implement `device_families` domain/persistence logic.
    /// USE: Called as `device_families` from handlers, repository impls, or sibling helpers.
    async fn device_families(&self) -> Result<Vec<String>, DeviceRepositoryError>;
    /// PURPOSE: Implement `device_types` domain/persistence logic.
    /// USE: Called as `device_types` from handlers, repository impls, or sibling helpers.
    async fn device_types(
        &self,
        device_family: Option<&str>,
    ) -> Result<Vec<String>, DeviceRepositoryError>;
    /// PURPOSE: Fetch a single record for `device by id`.
    /// USE: Detail handlers and repository lookups.
    async fn device_by_id(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `latest_heartbeat` domain/persistence logic.
    /// USE: Called as `latest_heartbeat` from handlers, repository impls, or sibling helpers.
    async fn latest_heartbeat(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `topology` domain/persistence logic.
    /// USE: Called as `topology` from handlers, repository impls, or sibling helpers.
    async fn topology(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: List/filter `list devices` rows for the API.
    /// USE: HTTP list handlers and repository trait methods.
    async fn list_devices(
        &self,
        query: &DeviceListQuery,
        is_admin: bool,
    ) -> Result<DeviceList, DeviceRepositoryError>;
}
