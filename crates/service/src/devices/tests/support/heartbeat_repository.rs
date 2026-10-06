use super::*;

#[async_trait]
impl HeartbeatRepository for RegistrationRepository {
    async fn record_controller_heartbeat(
        &self,
        heartbeat: &ControllerHeartbeat,
    ) -> Result<ControllerHeartbeatResult, DeviceRepositoryError> {
        self.callbacks
            .lock()
            .unwrap()
            .push(format!("heartbeat:{}:{}", heartbeat.uid, heartbeat.ip));
        Ok(ControllerHeartbeatResult {
            state_changed: false,
            alert: None,
        })
    }

    async fn record_device_heartbeat(
        &self,
        device_id: &str,
        heartbeat: &serde_json::Value,
    ) -> Result<Option<DeviceHeartbeatResult>, DeviceRepositoryError> {
        self.callbacks
            .lock()
            .unwrap()
            .push(format!("device-heartbeat:{device_id}"));
        Ok(Some(DeviceHeartbeatResult {
            state: "free".to_owned(),
            state_changed: false,
            interface_changes: Vec::new(),
            heartbeat_data: heartbeat.get("data").cloned().unwrap_or_default(),
            alerts: Vec::new(),
            post_update_events: Vec::new(),
            deferred_cancellations: Vec::new(),
        }))
    }
}
