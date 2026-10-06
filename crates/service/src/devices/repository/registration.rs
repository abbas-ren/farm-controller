use async_trait::async_trait;

use super::*;

/// Owns device/controller registration callbacks and execution completion callbacks.
#[async_trait]
pub(crate) trait DeviceRegistrationRepository: Send + Sync {
    async fn test_completion_target(
        &self,
        test_id: &str,
        device_id: &str,
    ) -> Result<Option<TestCompletionTarget>, DeviceRepositoryError>;
    async fn store_rtos_log_path(
        &self,
        test_id: &str,
        path: &str,
    ) -> Result<(), DeviceRepositoryError>;
    async fn mark_device_flashing(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceFlashingResult>, DeviceRepositoryError>;
    async fn export_devices(
        &self,
        query: &DeviceCsvQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn register_device(
        &self,
        registration: &DeviceRegistration,
    ) -> Result<DeviceRegistrationResult, DeviceRepositoryError>;
    async fn register_controller(
        &self,
        registration: &ControllerRegistration,
    ) -> Result<RegistrationResult, DeviceRepositoryError>;
    async fn save_gen5_mapping(
        &self,
        caller_ip: &str,
        mac: &str,
        status: CallbackStatus,
        tty_entry: Option<&TtyEntry>,
    ) -> Result<(), DeviceRepositoryError>;
    async fn confirm_flash(
        &self,
        device_type: &str,
        status: CallbackStatus,
    ) -> Result<FlashConfirmationResult, DeviceRepositoryError>;
}
