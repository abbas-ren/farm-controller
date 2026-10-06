use super::*;

mod cancellation;
mod creation;
mod logs;

#[async_trait]
impl ExecutionLifecycleRepository for PostgresDeviceRepository {
    async fn create_test_execution(
        &self,
        execution: &ExecutionCreation,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        self.create_test_execution_inner(execution).await
    }

    async fn test_execution_build_version(
        &self,
        test_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        self.test_execution_build_version_inner(test_id).await
    }

    async fn create_log(
        &self,
        request: &LogCreateRequest,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        self.create_log_inner(request).await
    }

    async fn list_logs(
        &self,
        query: &LogListQuery,
        search: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        self.list_logs_inner(query, search).await
    }

    async fn executions_by_build(
        &self,
        build_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        self.executions_by_build_inner(build_id, limit, offset)
            .await
    }

    async fn test_results(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        self.test_results_inner(test_id).await
    }

    async fn request_test_cancellation(
        &self,
        test_id: &str,
        user_id: &str,
        nfs_host_path: &str,
    ) -> Result<Option<TestCancellationResult>, DeviceRepositoryError> {
        self.request_test_cancellation_inner(test_id, user_id, nfs_host_path)
            .await
    }
}
