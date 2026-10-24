//! Alert listing and read-state capability trait.
//!
//! Implemented by Postgres and test fakes; handlers call via `DeviceRepository`.

use async_trait::async_trait;

use super::*;

/// Owns alert listing and read-state mutations.
#[async_trait]
pub(crate) trait AlertRepository: Send + Sync {
    /// PURPOSE: List admin-filtered alerts with sort/pagination.
    /// USE: Admin alerts HTTP handler.
    async fn admin_alerts(
        &self,
        page: i64,
        limit: i64,
        sort_by: &str,
        descending: bool,
    ) -> Result<AlertList, DeviceRepositoryError>;
    /// PURPOSE: List alerts in an optional date window with unread count.
    /// USE: All-alerts HTTP handler.
    async fn all_alerts(
        &self,
        page: i64,
        limit: i64,
        from: Option<chrono::DateTime<chrono::Utc>>,
        to: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<AlertList, DeviceRepositoryError>;
    /// PURPOSE: Mark one alert read by id.
    /// USE: Mark-read HTTP handler.
    async fn mark_alert_read(&self, alert_id: &str) -> Result<bool, DeviceRepositoryError>;
    /// PURPOSE: Mark every unread alert read.
    /// USE: Mark-all-read HTTP handler.
    async fn mark_all_alerts_read(&self) -> Result<(), DeviceRepositoryError>;
}
