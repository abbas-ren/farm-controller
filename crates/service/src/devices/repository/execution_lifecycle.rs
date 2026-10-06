use async_trait::async_trait;

use super::*;

/// Owns execution creation, cancellation, logs, and result lifecycle persistence.
#[async_trait]
pub(crate) trait ExecutionLifecycleRepository: Send + Sync {
    async fn create_test_execution(
        &self,
        execution: &ExecutionCreation,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn test_execution_build_version(
        &self,
        test_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError>;
    async fn create_log(
        &self,
        request: &LogCreateRequest,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn list_logs(
        &self,
        query: &LogListQuery,
        search: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn executions_by_build(
        &self,
        build_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn test_results(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn request_test_cancellation(
        &self,
        test_id: &str,
        user_id: &str,
        nfs_host_path: &str,
    ) -> Result<Option<TestCancellationResult>, DeviceRepositoryError>;
}
