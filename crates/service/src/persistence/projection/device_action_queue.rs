//! Joined row shape for dispatching device-action queue jobs.
//!
//! Combines `device_action_queue` columns with related test-execution fields
//! (`build_id`, `build_version`, `created_by`) so workers can claim a job and
//! start flash/test without a second round-trip. Populated via `FromQueryResult`
//! from custom SELECT statements in the action-queue service.

use sea_orm::FromQueryResult;

use crate::persistence::entity::device_action_queue;

/// One queued action plus execution context needed at claim/dispatch time.
#[derive(Debug, FromQueryResult)]
pub struct QueuedActionRow {
    pub id: uuid::Uuid,
    pub test_id: Option<String>,
    pub release_id: Option<uuid::Uuid>,
    pub version: Option<String>,
    pub device_type: String,
    pub user_id: String,
    pub mode: device_action_queue::QueueDispatchMode,
    pub target_device_id: Option<String>,
    pub ipl_flash_confirm: Option<bool>,

    /// From joined `test_executions` when `test_id` is present.
    pub build_id: Option<String>,
    pub build_version: Option<String>,
    pub created_by: Option<String>,
}
