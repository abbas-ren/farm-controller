//! In-memory `AnalyticsRepository` fake for device route tests.
//!
//! Stands in for production analytics queries behind `/api/v1/device/analytics/*`.

use super::*;

#[async_trait]
impl AnalyticsRepository for RegistrationRepository {
    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::device_state_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn device_state_analytics(&self) -> Result<DeviceStateAnalytics, DeviceRepositoryError> {
        Ok(DeviceStateAnalytics {
            total: 2,
            state_count: BTreeMap::from([("busy".to_owned(), 1), ("free".to_owned(), 1)]),
        })
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::detailed_device_state_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn detailed_device_state_analytics(
        &self,
    ) -> Result<DeviceStateDetailedAnalytics, DeviceRepositoryError> {
        Ok(DeviceStateDetailedAnalytics(BTreeMap::from([(
            "Gen5".to_owned(),
            vec![DeviceStateDetailedItem {
                device_id: "device-1".to_owned(),
                device_type: "Racer".to_owned(),
                state: "free".to_owned(),
            }],
        )])))
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::recent_execution_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn recent_execution_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "testId": "test-1",
            "buildVersion": "v1",
            "status": "completed",
            "testPlanName": "Smoke",
            "createdAt": "2026-10-02T00:00:00Z",
            "deviceType": "Racer",
            "startedAt": "2026-10-02T00:00:00Z",
            "endedAt": "2026-10-02T00:01:00Z",
            "totalTestCases": count.min(2),
            "createdBy": user_id,
        })])
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::execution_analytics_by_id`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn execution_analytics_by_id(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((test_id == "test-1").then(|| {
            serde_json::json!({
                "testId": test_id,
                "buildVersion": "v1",
                "status": "completed",
                "testPlanName": "Smoke",
                "createdAt": "2026-10-02T00:00:00Z",
                "totalTestCases": 2,
            })
        }))
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::daily_execution_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn daily_execution_analytics(
        &self,
        _user_id: &str,
        from: chrono::NaiveDate,
        _to: chrono::NaiveDate,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "date": from.to_string(),
            "totalExecutions": 1,
            "passedTestCases": 1,
            "failedTestCases": 1,
            "totalDurationSeconds": 60,
        })])
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::build_comparison_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn build_comparison_analytics(
        &self,
        _user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "buildId": "build-1", "buildVersion": "v1", "deviceType": "Racer",
            "averageDuration": 60000, "totalTestCases": count.min(2),
            "passedPercentage": 50.0,
        })])
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::build_comparison_analytics_by_id`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn build_comparison_analytics_by_id(
        &self,
        _user_id: &str,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((build_id == "build-1").then(|| {
            serde_json::json!({
                "buildId": build_id, "buildVersion": "v1", "deviceType": "Racer",
                "averageDuration": 60000, "totalTestCases": 2, "passedPercentage": 50.0,
            })
        }))
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::build_performance_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn build_performance_analytics(
        &self,
        _count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![build_performance_fixture("build-1")])
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::build_performance_analytics_by_id`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn build_performance_analytics_by_id(
        &self,
        build_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(build_performance_fixture(build_id))
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::device_usage_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn device_usage_analytics(
        &self,
        _start: chrono::DateTime<chrono::Utc>,
        _end: chrono::DateTime<chrono::Utc>,
        device_id: Option<&str>,
        device_family: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(serde_json::json!({
            "totalDevices": 1,
            "totalSeconds": 3600,
            "states": {"busy": 0, "not_reachable": 0, "free": 3600, "faulty": 0},
            "deviceId": device_id,
            "deviceFamily": device_family,
        }))
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::test_execution_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn test_execution_analytics(
        &self,
        _user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let counts = serde_json::json!({
            "total": 2, "passed": 1, "failed": 1, "inProgress": 0,
        });
        Ok(serde_json::json!({
            "allTime": counts,
            "weekly": [{"week": "28 Sep - 04 Oct", "total": 2,
                "passed": 1, "failed": 1, "inProgress": 0}],
            "monthly": [{"month": "Oct 2026", "total": 2,
                "passed": 1, "failed": 1, "inProgress": 0}],
        }))
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::in_progress_test_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn in_progress_test_analytics(
        &self,
        _user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "id": 1, "testPlanName": "Smoke", "createdAt": "2026-10-02T00:00:00Z",
            "startedAt": "2026-10-02T00:00:00Z", "endedAt": null,
            "status": "in_progress", "total": 2, "executed": 1,
        })])
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::test_plan_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn test_plan_analytics(
        &self,
        _user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(serde_json::json!({
            "total": 4, "completed": 1, "failed": 1,
            "inProgress": 1, "cancelled": 1,
        }))
    }

    /// PURPOSE: In-memory stand-in for `AnalyticsRepository::daily_test_analytics`.
    /// USE: device route tests exercising analytics persistence without a live database.
    async fn daily_test_analytics(
        &self,
        _user_id: &str,
        days: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "date": "2026-10-02", "totalExecutionSeconds": 60,
            "passed": days.min(1), "failed": 1,
        })])
    }
}
