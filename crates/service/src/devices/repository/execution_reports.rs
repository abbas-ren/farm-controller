use async_trait::async_trait;
use uuid::Uuid;

use super::*;

/// Owns execution report records and test-case report data.
#[async_trait]
pub(crate) trait ExecutionReportRepository: Send + Sync {
    async fn report_generation_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError>;
    async fn create_execution_report_record(
        &self,
        report_id: Uuid,
        test_id: &str,
        device_type: &str,
        build_version: &str,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn update_execution_report_status(
        &self,
        report_id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DeviceRepositoryError>;
    async fn execution_report(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn execution_report_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError>;
    async fn test_case_log_path(
        &self,
        case_id: i64,
    ) -> Result<Option<String>, DeviceRepositoryError>;
    async fn execution_cases(
        &self,
        user_id: &str,
        test_id: &str,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError>;
    async fn test_case(
        &self,
        case_id: i64,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
}
