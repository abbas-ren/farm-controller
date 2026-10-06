use async_trait::async_trait;

use super::*;

/// Owns alert listing and read-state mutations.
#[async_trait]
pub(crate) trait AlertRepository: Send + Sync {
    async fn admin_alerts(
        &self,
        page: i64,
        limit: i64,
        sort_by: &str,
        descending: bool,
    ) -> Result<AlertList, DeviceRepositoryError>;
    async fn all_alerts(
        &self,
        page: i64,
        limit: i64,
        from: Option<chrono::DateTime<chrono::Utc>>,
        to: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<AlertList, DeviceRepositoryError>;
    async fn mark_alert_read(&self, alert_id: &str) -> Result<bool, DeviceRepositoryError>;
    async fn mark_all_alerts_read(&self) -> Result<(), DeviceRepositoryError>;
}
