use super::*;

#[async_trait]
impl DeviceRegistrationRepository for PostgresDeviceRepository {
    async fn test_completion_target(
        &self,
        test_id: &str,
        device_id: &str,
    ) -> Result<Option<TestCompletionTarget>, DeviceRepositoryError> {
        test_completion_store::test_completion_target(&self.pool, test_id, device_id).await
    }

    async fn store_rtos_log_path(
        &self,
        test_id: &str,
        path: &str,
    ) -> Result<(), DeviceRepositoryError> {
        test_completion_store::store_rtos_log_path(&self.pool, test_id, path).await
    }

    async fn mark_device_flashing(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceFlashingResult>, DeviceRepositoryError> {
        flashing_store::mark_device_flashing(&self.pool, device_id).await
    }

    async fn export_devices(
        &self,
        query: &DeviceCsvQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        device_export_store::export_devices(&self.pool, query).await
    }

    async fn register_device(
        &self,
        registration: &DeviceRegistration,
    ) -> Result<repository_types::DeviceRegistrationResult, DeviceRepositoryError> {
        device_registration_store::register_device(&self.pool, registration).await
    }

    async fn register_controller(
        &self,
        registration: &ControllerRegistration,
    ) -> Result<RegistrationResult, DeviceRepositoryError> {
        registration_store::register_controller(&self.pool, registration).await
    }

    async fn save_gen5_mapping(
        &self,
        caller_ip: &str,
        mac: &str,
        status: CallbackStatus,
        tty_entry: Option<&TtyEntry>,
    ) -> Result<(), DeviceRepositoryError> {
        callback_store::save_gen5_mapping(&self.pool, caller_ip, mac, status, tty_entry).await
    }

    async fn confirm_flash(
        &self,
        device_type: &str,
        status: CallbackStatus,
    ) -> Result<FlashConfirmationResult, DeviceRepositoryError> {
        callback_store::confirm_flash(&self.pool, device_type, status).await
    }
}
