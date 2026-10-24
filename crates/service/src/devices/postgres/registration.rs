//! `DeviceRegistrationRepository` impl delegating to registration/flashing/export stores.

use super::*;

#[async_trait]
impl DeviceRegistrationRepository for PostgresDeviceRepository {
    /// PURPOSE: Resolve edge/controller targets for `test completion target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn test_completion_target(
        &self,
        test_id: &str,
        device_id: &str,
    ) -> Result<Option<TestCompletionTarget>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::registration", "test_completion_target");
        test_completion_store::test_completion_target(self.orm(), test_id, device_id).await
    }

    /// PURPOSE: Implement `store_rtos_log_path` domain/persistence logic.
    /// USE: Called as `store_rtos_log_path` from handlers, repository impls, or sibling helpers.
    async fn store_rtos_log_path(
        &self,
        test_id: &str,
        path: &str,
    ) -> Result<(), DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::registration", "store_rtos_log_path");
        test_completion_store::store_rtos_log_path(self.orm(), test_id, path).await
    }

    /// PURPOSE: Mutate persisted state for `mark device flashing`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn mark_device_flashing(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceFlashingResult>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::registration", "mark_device_flashing");
        flashing_store::mark_device_flashing(self.orm(), device_id).await
    }

    /// PURPOSE: Format/export `export devices` for download.
    /// USE: Export HTTP handlers.
    async fn export_devices(
        &self,
        query: &DeviceCsvQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::registration", "export_devices");
        device_export_store::export_devices(self.orm(), query).await
    }

    /// PURPOSE: Implement `register_device` domain/persistence logic.
    /// USE: Called as `register_device` from handlers, repository impls, or sibling helpers.
    async fn register_device(
        &self,
        registration: &DeviceRegistration,
    ) -> Result<repository_types::DeviceRegistrationResult, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::registration", "register_device");
        device_registration_store::register_device(self.orm(), registration).await
    }

    /// PURPOSE: Implement `register_controller` domain/persistence logic.
    /// USE: Called as `register_controller` from handlers, repository impls, or sibling helpers.
    async fn register_controller(
        &self,
        registration: &ControllerRegistration,
    ) -> Result<RegistrationResult, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::registration", "register_controller");
        registration_store::register_controller(self.orm(), registration).await
    }

    /// PURPOSE: Load or save `save gen5 mapping`.
    /// USE: Internal store helpers during multi-step transactions.
    async fn save_gen5_mapping(
        &self,
        caller_ip: &str,
        mac: &str,
        status: CallbackStatus,
        tty_entry: Option<&TtyEntry>,
    ) -> Result<(), DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::registration", "save_gen5_mapping");
        callback_store::save_gen5_mapping(self.orm(), caller_ip, mac, status, tty_entry).await
    }

    /// PURPOSE: Apply or confirm `confirm flash` configuration.
    /// USE: Relay configuration handlers and timeout workers.
    async fn confirm_flash(
        &self,
        device_type: &str,
        status: &IplFlashingStatus,
    ) -> Result<FlashConfirmationResult, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::registration", "confirm_flash");
        callback_store::confirm_flash(self.orm(), device_type, status).await
    }
}
