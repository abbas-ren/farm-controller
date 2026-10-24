//! Analytics capability trait (device state, executions, builds, usage).
//!
//! Implemented by Postgres; mirrors Node `AnalyticsService` surface.

use async_trait::async_trait;

use super::*;

/// Owns aggregate device, execution, build, and usage analytics queries.
#[async_trait]
pub(crate) trait AnalyticsRepository: Send + Sync {
    /// PURPOSE: Aggregate device counts by state.
    /// USE: Device-state analytics handler.
    async fn device_state_analytics(&self) -> Result<DeviceStateAnalytics, DeviceRepositoryError>;
    /// PURPOSE: Per-device state detail for analytics.
    /// USE: Detailed device-state handler.
    async fn detailed_device_state_analytics(
        &self,
    ) -> Result<DeviceStateDetailedAnalytics, DeviceRepositoryError>;
    /// PURPOSE: Recent executions for a user.
    /// USE: Recent-executions analytics handler.
    async fn recent_execution_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Single execution analytics payload.
    /// USE: Execution-by-id analytics handler.
    async fn execution_analytics_by_id(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Daily execution rollups for a date range.
    /// USE: Daily-executions analytics handler.
    async fn daily_execution_analytics(
        &self,
        user_id: &str,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Compare recent builds for a user.
    /// USE: Build-comparison analytics handler.
    async fn build_comparison_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Build comparison for one build id.
    /// USE: Build-comparison-by-id handler.
    async fn build_comparison_analytics_by_id(
        &self,
        user_id: &str,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Build performance rollups.
    /// USE: Build-performance analytics handler.
    async fn build_performance_analytics(
        &self,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Performance for one build.
    /// USE: Build-performance-by-id handler.
    async fn build_performance_analytics_by_id(
        &self,
        build_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    /// PURPOSE: Device usage within a time window.
    /// USE: Device-usage analytics handler.
    async fn device_usage_analytics(
        &self,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
        device_id: Option<&str>,
        device_family: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    /// PURPOSE: User test-execution summary analytics.
    /// USE: Test-analytics handler.
    async fn test_execution_analytics(
        &self,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    /// PURPOSE: In-progress executions for a user.
    /// USE: In-progress analytics handler.
    async fn in_progress_test_analytics(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Test-plan summary analytics for a user.
    /// USE: Test-plan analytics handler.
    async fn test_plan_analytics(
        &self,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    /// PURPOSE: Daily test summary for N days.
    /// USE: Daily-test-summary handler.
    async fn daily_test_analytics(
        &self,
        user_id: &str,
        days: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
}
