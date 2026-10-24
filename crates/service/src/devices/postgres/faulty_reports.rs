//! `FaultyReportRepository` impl delegating to `faulty_report_store`.

use super::*;

#[async_trait]
impl FaultyReportRepository for PostgresDeviceRepository {
    /// PURPOSE: Create/persist `create faulty report`.
    /// USE: Create handlers after validation.
    async fn create_faulty_report(
        &self,
        request: &FaultyReportCreate,
    ) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::faulty_reports", "create_faulty_report");
        faulty_report_store::create(self.orm(), request).await
    }

    /// PURPOSE: Implement `finalize_faulty_report` domain/persistence logic.
    /// USE: Called as `finalize_faulty_report` from handlers, repository impls, or sibling helpers.
    async fn finalize_faulty_report(
        &self,
        report_id: Uuid,
        file_path: Option<&str>,
        logs_path: Option<&str>,
    ) -> Result<repository_types::FaultyReportFinalization, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::faulty_reports", "finalize_faulty_report");
        faulty_report_store::finalize(self.orm(), report_id, file_path, logs_path).await
    }

    /// PURPOSE: Implement `faulty_reports` domain/persistence logic.
    /// USE: Called as `faulty_reports` from handlers, repository impls, or sibling helpers.
    async fn faulty_reports(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::faulty_reports", "faulty_reports");
        faulty_report_store::list(self.orm()).await
    }

    /// PURPOSE: Fetch a single record for `faulty report by id`.
    /// USE: Detail handlers and repository lookups.
    async fn faulty_report_by_id(
        &self,
        report_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::faulty_reports", "faulty_report_by_id");
        faulty_report_store::by_id(self.orm(), report_id).await
    }

    /// PURPOSE: Mutate persisted state for `update faulty report status`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn update_faulty_report_status(
        &self,
        report_id: Uuid,
        status: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::faulty_reports", "update_faulty_report_status");
        faulty_report_store::update_status(self.orm(), report_id, status).await
    }

    /// PURPOSE: Delete `delete faulty report` from persistence.
    /// USE: Delete handlers after edge cleanup when required.
    async fn delete_faulty_report(&self, report_id: Uuid) -> Result<bool, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::faulty_reports", "delete_faulty_report");
        faulty_report_store::delete(self.orm(), report_id).await
    }
}

#[async_trait]
impl HeartbeatRepository for PostgresDeviceRepository {
    /// PURPOSE: Persist a heartbeat/event for `record controller heartbeat`.
    /// USE: Heartbeat workers and callback handlers.
    async fn record_controller_heartbeat(
        &self,
        heartbeat: &ControllerHeartbeat,
    ) -> Result<ControllerHeartbeatResult, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::faulty_reports", "record_controller_heartbeat");
        heartbeat_store::record_controller(self.orm(), heartbeat).await
    }

    /// PURPOSE: Persist a heartbeat/event for `record device heartbeat`.
    /// USE: Heartbeat workers and callback handlers.
    async fn record_device_heartbeat(
        &self,
        device_id: &str,
        heartbeat: &serde_json::Value,
    ) -> Result<Option<DeviceHeartbeatResult>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::faulty_reports", "record_device_heartbeat");
        heartbeat_store::record_device(self.orm(), device_id, heartbeat).await
    }
}
