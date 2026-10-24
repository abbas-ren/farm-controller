//! Faulty-report CRUD capability trait (Node `FaultyReportService`).

use async_trait::async_trait;
use uuid::Uuid;

use super::*;

/// Owns faulty-report creation, finalization, status, and retrieval.
#[async_trait]
pub(crate) trait FaultyReportRepository: Send + Sync {
    /// PURPOSE: Create/persist `create faulty report`.
    /// USE: Create handlers after validation.
    async fn create_faulty_report(
        &self,
        request: &FaultyReportCreate,
    ) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError>;
    /// PURPOSE: Implement `finalize_faulty_report` domain/persistence logic.
    /// USE: Called as `finalize_faulty_report` from handlers, repository impls, or sibling helpers.
    async fn finalize_faulty_report(
        &self,
        report_id: Uuid,
        file_path: Option<&str>,
        logs_path: Option<&str>,
    ) -> Result<FaultyReportFinalization, DeviceRepositoryError>;
    /// PURPOSE: Implement `faulty_reports` domain/persistence logic.
    /// USE: Called as `faulty_reports` from handlers, repository impls, or sibling helpers.
    async fn faulty_reports(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Fetch a single record for `faulty report by id`.
    /// USE: Detail handlers and repository lookups.
    async fn faulty_report_by_id(
        &self,
        report_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Mutate persisted state for `update faulty report status`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn update_faulty_report_status(
        &self,
        report_id: Uuid,
        status: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Delete `delete faulty report` from persistence.
    /// USE: Delete handlers after edge cleanup when required.
    async fn delete_faulty_report(&self, report_id: Uuid) -> Result<bool, DeviceRepositoryError>;
}
