//! Test execution list/detail/export capability trait.

use async_trait::async_trait;

use super::*;

/// Owns execution listing, detail, summary, and export queries.
#[async_trait]
pub(crate) trait ExecutionQueryRepository: Send + Sync {
    /// PURPOSE: List/filter `list test executions` rows for the API.
    /// USE: HTTP list handlers and repository trait methods.
    async fn list_test_executions(
        &self,
        user_id: &str,
        query: &ExecutionListQuery,
    ) -> Result<ExecutionList, DeviceRepositoryError>;
    /// PURPOSE: Implement `test_execution_by_device` domain/persistence logic.
    /// USE: Called as `test_execution_by_device` from handlers, repository impls, or sibling helpers.
    async fn test_execution_by_device(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `in_progress_test_executions` domain/persistence logic.
    /// USE: Called as `in_progress_test_executions` from handlers, repository impls, or sibling helpers.
    async fn in_progress_test_executions(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `single_test_execution` domain/persistence logic.
    /// USE: Called as `single_test_execution` from handlers, repository impls, or sibling helpers.
    async fn single_test_execution(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `execution_case_ids` domain/persistence logic.
    /// USE: Called as `execution_case_ids` from handlers, repository impls, or sibling helpers.
    async fn execution_case_ids(
        &self,
        test_id: &str,
    ) -> Result<Option<Vec<String>>, DeviceRepositoryError>;
    /// PURPOSE: Implement `test_execution` domain/persistence logic.
    /// USE: Called as `test_execution` from handlers, repository impls, or sibling helpers.
    async fn test_execution(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `test_execution_summary` domain/persistence logic.
    /// USE: Called as `test_execution_summary` from handlers, repository impls, or sibling helpers.
    async fn test_execution_summary(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `test_export_rows` domain/persistence logic.
    /// USE: Called as `test_export_rows` from handlers, repository impls, or sibling helpers.
    async fn test_export_rows(
        &self,
        user_id: &str,
    ) -> Result<Vec<TestExportRow>, DeviceRepositoryError>;
}
