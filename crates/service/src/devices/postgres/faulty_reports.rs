use super::*;

#[async_trait]
impl FaultyReportRepository for PostgresDeviceRepository {
    async fn create_faulty_report(
        &self,
        request: &FaultyReportCreate,
    ) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError> {
        faulty_report_store::create(&self.pool, request).await
    }

    async fn finalize_faulty_report(
        &self,
        report_id: Uuid,
        file_path: Option<&str>,
        logs_path: Option<&str>,
    ) -> Result<repository_types::FaultyReportFinalization, DeviceRepositoryError> {
        faulty_report_store::finalize(&self.pool, report_id, file_path, logs_path).await
    }

    async fn faulty_reports(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        faulty_report_store::list(&self.pool).await
    }

    async fn faulty_report_by_id(
        &self,
        report_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        faulty_report_store::by_id(&self.pool, report_id).await
    }

    async fn update_faulty_report_status(
        &self,
        report_id: Uuid,
        status: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        faulty_report_store::update_status(&self.pool, report_id, status).await
    }

    async fn delete_faulty_report(&self, report_id: Uuid) -> Result<bool, DeviceRepositoryError> {
        faulty_report_store::delete(&self.pool, report_id).await
    }
}

#[async_trait]
impl HeartbeatRepository for PostgresDeviceRepository {
    async fn record_controller_heartbeat(
        &self,
        heartbeat: &ControllerHeartbeat,
    ) -> Result<ControllerHeartbeatResult, DeviceRepositoryError> {
        heartbeat_store::record_controller(&self.pool, heartbeat).await
    }

    async fn record_device_heartbeat(
        &self,
        device_id: &str,
        heartbeat: &serde_json::Value,
    ) -> Result<Option<DeviceHeartbeatResult>, DeviceRepositoryError> {
        heartbeat_store::record_device(&self.pool, device_id, heartbeat).await
    }
}
