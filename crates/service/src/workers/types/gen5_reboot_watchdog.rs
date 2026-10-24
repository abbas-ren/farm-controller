use chrono::{DateTime, Utc};

use crate::socket::types::farm::ServerEvent;

#[derive(Debug, sea_orm::FromQueryResult)]
pub struct StuckDevice {
    pub device_id: String,
    pub attempted_at: Option<DateTime<Utc>>,
    pub controller_ip: Option<String>,
    pub power_port: Option<String>,
}

#[derive(Debug, sea_orm::FromQueryResult)]
pub struct FailedTest {
    pub test_id: String,
    pub build_id: Option<String>,
    pub user_id: Option<String>,
}

pub enum WatchdogDecision {
    Reboot,
    Wait,
    Fail,
}

pub struct WatchdogTransition {
    pub reboot: Option<(String, String)>,
    pub events: Vec<ServerEvent>,
}
