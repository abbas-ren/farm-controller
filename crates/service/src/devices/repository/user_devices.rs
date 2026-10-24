//! User-scoped device listing and heartbeat-timeout capability trait.

use async_trait::async_trait;

use super::*;

/// Owns user-facing device listings, active device listings, and heartbeat timeout updates.
#[async_trait]
pub(crate) trait UserDeviceRepository: Send + Sync {
    /// PURPOSE: List/filter `list user devices` rows for the API.
    /// USE: HTTP list handlers and repository trait methods.
    async fn list_user_devices(
        &self,
        query: &UserDeviceListQuery,
    ) -> Result<DeviceList, DeviceRepositoryError>;
    /// PURPOSE: Implement `active_devices` domain/persistence logic.
    /// USE: Called as `active_devices` from handlers, repository impls, or sibling helpers.
    async fn active_devices(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Mutate persisted state for `update heartbeat timeout`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn update_heartbeat_timeout(
        &self,
        device_id: &str,
        value: i64,
    ) -> Result<bool, DeviceRepositoryError>;
}
