use super::*;

#[async_trait]
impl AnalyticsRepository for PostgresDeviceRepository {
    async fn device_state_analytics(&self) -> Result<DeviceStateAnalytics, DeviceRepositoryError> {
        analytics_store::device_state_counts(&self.pool).await
    }

    async fn detailed_device_state_analytics(
        &self,
    ) -> Result<DeviceStateDetailedAnalytics, DeviceRepositoryError> {
        analytics_store::device_state_details(&self.pool).await
    }

    async fn recent_execution_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::recent_executions(&self.pool, user_id, count).await
    }

    async fn execution_analytics_by_id(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::execution_by_id(&self.pool, test_id).await
    }

    async fn daily_execution_analytics(
        &self,
        user_id: &str,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::execution_daily(&self.pool, user_id, from, to).await
    }

    async fn build_comparison_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::build_comparisons(&self.pool, user_id, count).await
    }

    async fn build_comparison_analytics_by_id(
        &self,
        user_id: &str,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::build_comparison_by_id(&self.pool, user_id, build_id).await
    }

    async fn build_performance_analytics(
        &self,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::build_performance(&self.pool, count).await
    }

    async fn build_performance_analytics_by_id(
        &self,
        build_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        analytics_store::build_performance_by_id(&self.pool, build_id).await
    }

    async fn device_usage_analytics(
        &self,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
        device_id: Option<&str>,
        device_family: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        analytics_store::device_usage(&self.pool, start, end, device_id, device_family).await
    }

    async fn test_execution_analytics(
        &self,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        analytics_store::test_execution_analytics(&self.pool, user_id).await
    }

    async fn in_progress_test_analytics(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::in_progress_test_analytics(&self.pool, user_id).await
    }

    async fn test_plan_analytics(
        &self,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        analytics_store::test_plan_summary(&self.pool, user_id).await
    }

    async fn daily_test_analytics(
        &self,
        user_id: &str,
        days: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::daily_test_summary(&self.pool, user_id, days).await
    }
}

#[async_trait]
impl AlertRepository for PostgresDeviceRepository {
    async fn admin_alerts(
        &self,
        page: i64,
        limit: i64,
        sort_by: &str,
        descending: bool,
    ) -> Result<AlertList, DeviceRepositoryError> {
        alert_store::list_admin(&self.pool, page, limit, sort_by, descending).await
    }

    async fn all_alerts(
        &self,
        page: i64,
        limit: i64,
        from: Option<chrono::DateTime<chrono::Utc>>,
        to: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<AlertList, DeviceRepositoryError> {
        alert_store::list_all(&self.pool, page, limit, from, to).await
    }

    async fn mark_alert_read(&self, alert_id: &str) -> Result<bool, DeviceRepositoryError> {
        alert_store::mark_read(&self.pool, alert_id).await
    }

    async fn mark_all_alerts_read(&self) -> Result<(), DeviceRepositoryError> {
        alert_store::mark_all_read(&self.pool).await
    }
}
