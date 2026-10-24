//! Execution report record and case-query capability trait.

use async_trait::async_trait;
use uuid::Uuid;

use super::*;

/// Owns execution report records and test-case report data.
#[async_trait]
pub(crate) trait ExecutionReportRepository: Send + Sync {
    /// PURPOSE: Resolve edge/controller targets for `report generation target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn report_generation_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError>;
    /// PURPOSE: Create/persist `create execution report record`.
    /// USE: Create handlers after validation.
    async fn create_execution_report_record(
        &self,
        report_id: Uuid,
        test_id: &str,
        device_type: &str,
        build_version: &str,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    /// PURPOSE: Mutate persisted state for `update execution report status`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn update_execution_report_status(
        &self,
        report_id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DeviceRepositoryError>;
    /// PURPOSE: Implement `execution_report` domain/persistence logic.
    /// USE: Called as `execution_report` from handlers, repository impls, or sibling helpers.
    async fn execution_report(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Resolve edge/controller targets for `execution report target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn execution_report_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError>;
    /// PURPOSE: Implement `test_case_log_path` domain/persistence logic.
    /// USE: Called as `test_case_log_path` from handlers, repository impls, or sibling helpers.
    async fn test_case_log_path(
        &self,
        case_id: i64,
    ) -> Result<Option<String>, DeviceRepositoryError>;
    /// PURPOSE: Implement `execution_cases` domain/persistence logic.
    /// USE: Called as `execution_cases` from handlers, repository impls, or sibling helpers.
    async fn execution_cases(
        &self,
        user_id: &str,
        test_id: &str,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError>;
    /// PURPOSE: Implement `test_case` domain/persistence logic.
    /// USE: Called as `test_case` from handlers, repository impls, or sibling helpers.
    async fn test_case(
        &self,
        case_id: i64,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
}
