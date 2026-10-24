//! Claim and dispatch shapes for the device-action queue worker.
//!
//! Mirrors the legacy `DeviceActionQueueService` job payload: a queued row plus
//! the device/controller chosen under `FOR UPDATE SKIP LOCKED`.

use uuid::Uuid;

use crate::persistence::entity::device_action_queue;

/// Row projected from `device_action_queue` (+ joined test/release fields) while claiming.
#[derive(Debug, sea_orm::FromQueryResult)]
pub struct QueuedAction {
    pub id: Uuid,
    pub test_id: Option<String>,
    pub release_id: Option<Uuid>,
    pub version: Option<String>,
    pub device_type: String,
    pub user_id: String,
    pub mode: device_action_queue::QueueDispatchMode,
    pub target_device_id: Option<String>,
    /// When true, this is a post-IPL re-dispatch rather than a fresh queue claim.
    pub ipl_flash_confirm: Option<bool>,
    pub build_id: Option<String>,
    pub build_version: Option<String>,
    pub created_by: Option<String>,
}

/// Queued action after a free device (and its controller IP) has been assigned.
pub struct AssignedAction {
    pub action: QueuedAction,
    pub device_id: String,
    pub ip_address: String,
}

/// Gen4 IPL flash target resolved from relay / GPIO mappings on the controller.
#[derive(Debug, sea_orm::FromQueryResult)]
pub struct Gen4IplTarget {
    pub controller_ip: String,
    pub relay_serial: String,
    pub channel_number: i32,
    pub device_mac: String,
    pub gpio: String,
    pub gpio_default_level: String,
    pub relay_default_level: String,
}
