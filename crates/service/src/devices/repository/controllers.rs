use async_trait::async_trait;

use super::*;

/// Owns controller-facing device actions, controller listings, and controller lifecycle.
#[async_trait]
pub(crate) trait ControllerRepository: Send + Sync {
    async fn device_action_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceActionTarget>, DeviceRepositoryError>;
    async fn apply_device_action(
        &self,
        device_id: &str,
        action: DeviceAction,
        user_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn list_controllers(
        &self,
        query: &ControllerListQuery,
    ) -> Result<ControllerList, DeviceRepositoryError>;
    async fn edit_controller(
        &self,
        controller_id: &str,
        request: &ControllerEditRequest,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn controller_delete_target(
        &self,
        controller_id: &str,
    ) -> Result<Option<ControllerDeleteTarget>, DeviceRepositoryError>;
    async fn delete_controller(&self, controller_id: &str) -> Result<bool, DeviceRepositoryError>;
    async fn device_delete_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceDeleteTarget>, DeviceRepositoryError>;
    async fn delete_device(&self, device_id: &str) -> Result<bool, DeviceRepositoryError>;
}
