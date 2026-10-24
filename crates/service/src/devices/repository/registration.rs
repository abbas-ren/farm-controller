//! Registration, flashing, export, and Gen5 mapping capability trait.

use async_trait::async_trait;

use super::*;
use crate::devices::IplFlashingStatus;

/// Owns device/controller registration callbacks and execution completion callbacks.
#[async_trait]
pub(crate) trait DeviceRegistrationRepository: Send + Sync {
    /// PURPOSE: Resolve edge/controller targets for `test completion target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn test_completion_target(
        &self,
        test_id: &str,
        device_id: &str,
    ) -> Result<Option<TestCompletionTarget>, DeviceRepositoryError>;
    /// PURPOSE: Implement `store_rtos_log_path` domain/persistence logic.
    /// USE: Called as `store_rtos_log_path` from handlers, repository impls, or sibling helpers.
    async fn store_rtos_log_path(
        &self,
        test_id: &str,
        path: &str,
    ) -> Result<(), DeviceRepositoryError>;
    /// PURPOSE: Mutate persisted state for `mark device flashing`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn mark_device_flashing(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceFlashingResult>, DeviceRepositoryError>;
    /// PURPOSE: Format/export `export devices` for download.
    /// USE: Export HTTP handlers.
    async fn export_devices(
        &self,
        query: &DeviceCsvQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `register_device` domain/persistence logic.
    /// USE: Called as `register_device` from handlers, repository impls, or sibling helpers.
    async fn register_device(
        &self,
        registration: &DeviceRegistration,
    ) -> Result<DeviceRegistrationResult, DeviceRepositoryError>;
    /// PURPOSE: Implement `register_controller` domain/persistence logic.
    /// USE: Called as `register_controller` from handlers, repository impls, or sibling helpers.
    async fn register_controller(
        &self,
        registration: &ControllerRegistration,
    ) -> Result<RegistrationResult, DeviceRepositoryError>;
    /// PURPOSE: Load or save `save gen5 mapping`.
    /// USE: Internal store helpers during multi-step transactions.
    async fn save_gen5_mapping(
        &self,
        caller_ip: &str,
        mac: &str,
        status: CallbackStatus,
        tty_entry: Option<&TtyEntry>,
    ) -> Result<(), DeviceRepositoryError>;
    /// PURPOSE: Apply or confirm `confirm flash` configuration.
    /// USE: Relay configuration handlers and timeout workers.
    async fn confirm_flash(
        &self,
        device_type: &str,
        flashing_status: &IplFlashingStatus,
    ) -> Result<FlashConfirmationResult, DeviceRepositoryError>;
}
