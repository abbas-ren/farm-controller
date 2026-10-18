use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct QueuedAction {
    pub id: Uuid,
    pub test_id: Option<String>,
    pub release_id: Option<Uuid>,
    pub version: Option<String>,
    pub device_type: String,
    pub user_id: String,
    pub mode: String,
    pub target_device_id: Option<String>,
    pub ipl_flash_confirm: Option<bool>,
    pub build_id: Option<String>,
    pub build_version: Option<String>,
    pub created_by: Option<String>,
}

pub struct AssignedAction {
    pub action: QueuedAction,
    pub device_id: String,
    pub ip_address: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct Gen4IplTarget {
    pub controller_ip: String,
    pub relay_serial: String,
    pub channel_number: i32,
    pub device_mac: String,
    pub gpio: String,
    pub gpio_default_level: String,
    pub relay_default_level: String,
}
