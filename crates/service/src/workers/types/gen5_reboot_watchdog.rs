//! DTOs for the Gen5 reboot watchdog (legacy `Gen5RebootWatchdogService`).

use chrono::{DateTime, Utc};

use crate::socket::types::farm::ServerEvent;

/// Gen5 device stuck mid-flash/upgrade without a recent heartbeat.
#[derive(Debug, sea_orm::FromQueryResult)]
pub struct StuckDevice {
    pub device_id: String,
    /// Prior reboot attempt timestamp; `None` means first detection.
    pub attempted_at: Option<DateTime<Utc>>,
    pub controller_ip: Option<String>,
    pub power_port: Option<String>,
}

/// Test execution that must be failed when a second reboot timeout fires.
#[derive(Debug, sea_orm::FromQueryResult)]
pub struct FailedTest {
    pub test_id: String,
    pub build_id: Option<String>,
    pub user_id: Option<String>,
}

/// Decision for one stuck device on a watchdog tick.
pub enum WatchdogDecision {
    /// Send a one-shot reboot via the device controller.
    Reboot,
    /// Already rebooted recently; wait for another timeout window.
    Wait,
    /// Second timeout — fail the job / test.
    Fail,
}

/// Aggregated side effects after applying a watchdog decision.
pub struct WatchdogTransition {
    /// `(controller_ip, device_id)` when a reboot command should be sent.
    pub reboot: Option<(String, String)>,
    pub events: Vec<ServerEvent>,
}
