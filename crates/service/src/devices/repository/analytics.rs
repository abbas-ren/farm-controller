use async_trait::async_trait;

use super::*;

/// Owns aggregate device, execution, build, and usage analytics queries.
#[async_trait]
pub(crate) trait AnalyticsRepository: Send + Sync {
    async fn device_state_analytics(&self) -> Result<DeviceStateAnalytics, DeviceRepositoryError>;
    async fn detailed_device_state_analytics(
        &self,
    ) -> Result<DeviceStateDetailedAnalytics, DeviceRepositoryError>;
    async fn recent_execution_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn execution_analytics_by_id(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn daily_execution_analytics(
        &self,
        user_id: &str,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn build_comparison_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn build_comparison_analytics_by_id(
        &self,
        user_id: &str,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn build_performance_analytics(
        &self,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn build_performance_analytics_by_id(
        &self,
        build_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn device_usage_analytics(
        &self,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
        device_id: Option<&str>,
        device_family: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn test_execution_analytics(
        &self,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn in_progress_test_analytics(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn test_plan_analytics(
        &self,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn daily_test_analytics(
        &self,
        user_id: &str,
        days: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
}
