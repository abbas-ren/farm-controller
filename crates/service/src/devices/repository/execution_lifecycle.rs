//! Test execution create/cancel/logs/status capability trait.
//!
//! Mirrors Node `TestService` / `LogService` lifecycle methods.

use async_trait::async_trait;

use super::*;

/// Owns execution creation, cancellation, logs, and result lifecycle persistence.
#[async_trait]
pub(crate) trait ExecutionLifecycleRepository: Send + Sync {
    /// PURPOSE: Create/persist `create test execution`.
    /// USE: Create handlers after validation.
    async fn create_test_execution(
        &self,
        execution: &ExecutionCreation,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    /// PURPOSE: Implement `test_execution_build_version` domain/persistence logic.
    /// USE: Called as `test_execution_build_version` from handlers, repository impls, or sibling helpers.
    async fn test_execution_build_version(
        &self,
        test_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError>;
    /// PURPOSE: Create/persist `create log`.
    /// USE: Create handlers after validation.
    async fn create_log(
        &self,
        request: &LogCreateRequest,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    /// PURPOSE: List/filter `list logs` rows for the API.
    /// USE: HTTP list handlers and repository trait methods.
    async fn list_logs(
        &self,
        query: &LogListQuery,
        search: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    /// PURPOSE: Implement `executions_by_build` domain/persistence logic.
    /// USE: Called as `executions_by_build` from handlers, repository impls, or sibling helpers.
    async fn executions_by_build(
        &self,
        build_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    /// PURPOSE: Implement `test_results` domain/persistence logic.
    /// USE: Called as `test_results` from handlers, repository impls, or sibling helpers.
    async fn test_results(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Mutate persisted state for `update execution status`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn update_execution_status(
        &self,
        test_id: &str,
        status: &str,
    ) -> Result<bool, DeviceRepositoryError>;
    /// PURPOSE: Implement `request_test_cancellation` domain/persistence logic.
    /// USE: Called as `request_test_cancellation` from handlers, repository impls, or sibling helpers.
    async fn request_test_cancellation(
        &self,
        test_id: &str,
        user_id: &str,
        nfs_host_path: &str,
    ) -> Result<Option<TestCancellationResult>, DeviceRepositoryError>;
}
