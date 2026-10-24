use sea_orm::FromQueryResult;

use crate::persistence::entity::device_action_queue;

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

    pub build_id: Option<String>,
    pub build_version: Option<String>,
    pub created_by: Option<String>,
}
