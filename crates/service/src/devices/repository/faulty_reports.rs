use async_trait::async_trait;
use uuid::Uuid;

use super::*;

/// Owns faulty-report creation, finalization, status, and retrieval.
#[async_trait]
pub(crate) trait FaultyReportRepository: Send + Sync {
    async fn create_faulty_report(
        &self,
        request: &FaultyReportCreate,
    ) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError>;
    async fn finalize_faulty_report(
        &self,
        report_id: Uuid,
        file_path: Option<&str>,
        logs_path: Option<&str>,
    ) -> Result<FaultyReportFinalization, DeviceRepositoryError>;
    async fn faulty_reports(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn faulty_report_by_id(
        &self,
        report_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn update_faulty_report_status(
        &self,
        report_id: Uuid,
        status: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn delete_faulty_report(&self, report_id: Uuid) -> Result<bool, DeviceRepositoryError>;
}
