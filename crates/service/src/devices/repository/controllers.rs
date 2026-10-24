//! Controller and device-action capability trait.
//!
//! Implemented by Postgres; mirrors Node `DeviceService` controller APIs.

use async_trait::async_trait;

use super::*;

/// Owns controller-facing device actions, controller listings, and controller lifecycle.
#[async_trait]
pub(crate) trait ControllerRepository: Send + Sync {
    /// PURPOSE: Resolve edge/controller targets for `device action target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn device_action_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceActionTarget>, DeviceRepositoryError>;
    /// PURPOSE: Mutate persisted state for `apply device action`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn apply_device_action(
        &self,
        device_id: &str,
        action: DeviceAction,
        user_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: List/filter `list controllers` rows for the API.
    /// USE: HTTP list handlers and repository trait methods.
    async fn list_controllers(
        &self,
        query: &ControllerListQuery,
    ) -> Result<ControllerList, DeviceRepositoryError>;
    /// PURPOSE: Implement `edit_controller` domain/persistence logic.
    /// USE: Called as `edit_controller` from handlers, repository impls, or sibling helpers.
    async fn edit_controller(
        &self,
        controller_id: &str,
        request: &ControllerEditRequest,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Resolve edge/controller targets for `controller delete target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn controller_delete_target(
        &self,
        controller_id: &str,
    ) -> Result<Option<ControllerDeleteTarget>, DeviceRepositoryError>;
    /// PURPOSE: Delete `delete controller` from persistence.
    /// USE: Delete handlers after edge cleanup when required.
    async fn delete_controller(&self, controller_id: &str) -> Result<bool, DeviceRepositoryError>;
    /// PURPOSE: Resolve edge/controller targets for `device delete target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn device_delete_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceDeleteTarget>, DeviceRepositoryError>;
    /// PURPOSE: Delete `delete device` from persistence.
    /// USE: Delete handlers after edge cleanup when required.
    async fn delete_device(&self, device_id: &str) -> Result<bool, DeviceRepositoryError>;
}
