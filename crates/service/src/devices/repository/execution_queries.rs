use async_trait::async_trait;

use super::*;

/// Owns execution listing, detail, summary, and export queries.
#[async_trait]
pub(crate) trait ExecutionQueryRepository: Send + Sync {
    async fn list_test_executions(
        &self,
        user_id: &str,
        query: &ExecutionListQuery,
    ) -> Result<ExecutionList, DeviceRepositoryError>;
    async fn test_execution_by_device(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn in_progress_test_executions(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn single_test_execution(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn execution_case_ids(
        &self,
        test_id: &str,
    ) -> Result<Option<Vec<String>>, DeviceRepositoryError>;
    async fn test_execution(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn test_execution_summary(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn test_export_rows(
        &self,
        user_id: &str,
    ) -> Result<Vec<TestExportRow>, DeviceRepositoryError>;
}
