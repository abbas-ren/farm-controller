use super::*;

#[async_trait]
impl AlertRepository for RegistrationRepository {
    async fn admin_alerts(
        &self,
        page: i64,
        _limit: i64,
        _sort_by: &str,
        _descending: bool,
    ) -> Result<AlertList, DeviceRepositoryError> {
        Ok(AlertList {
            data: vec![serde_json::json!({
                "id": "alert-1", "userId": "user-1", "title": "Device unavailable",
                "message": "Device device-1 is unavailable", "data": {"deviceId": "device-1"},
                "type": "warning", "status": "unread", "isRead": false,
                "readAt": null, "createdAt": "2026-10-02T00:00:00Z",
                "updatedAt": "2026-10-02T00:00:00Z", "deviceId": "device-1",
            })],
            total_data: 1,
            total_pages: 1,
            current_page: page,
            total_unread_count: None,
        })
    }

    async fn all_alerts(
        &self,
        page: i64,
        _limit: i64,
        _from: Option<chrono::DateTime<chrono::Utc>>,
        _to: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<AlertList, DeviceRepositoryError> {
        Ok(AlertList {
            data: vec![serde_json::json!({"id": "alert-1", "isRead": false})],
            total_data: 1,
            total_pages: 1,
            current_page: page,
            total_unread_count: Some(1),
        })
    }

    async fn mark_alert_read(&self, alert_id: &str) -> Result<bool, DeviceRepositoryError> {
        Ok(alert_id != "missing")
    }

    async fn mark_all_alerts_read(&self) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }
}
