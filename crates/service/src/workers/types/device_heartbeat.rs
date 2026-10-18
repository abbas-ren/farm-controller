use super::common::DeviceState;

#[derive(Debug, sqlx::FromRow)]
pub struct StaleDevice {
    pub device_id: String,
    pub previous_state: String,
    pub target_state: String,
    pub timeout: i32,
}

pub struct DeviceStateTransition {
    pub device_id: String,
    pub state: DeviceState,
    pub alert: serde_json::Value,
}
