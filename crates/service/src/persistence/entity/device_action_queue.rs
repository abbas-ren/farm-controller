//! Device action queue jobs (`device_action_queue` table).
//!
//! Ports `farm_controller_old/device-service/src/models/DeviceActionQueue.ts` and
//! the logic behind `DeviceActionQueueService`. Rows schedule flash or test work
//! for a device type, bulk set, or specific DUT. Retention deletes only terminal
//! `completed` / `cancelled` jobs past the retention window.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Kind of work the queue entry represents.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_device_action_queue_type"
)]
pub enum QueueJobType {
    /// Run automated tests after device prep.
    #[sea_orm(string_value = "test")]
    Test,
    /// Flash a release/build onto the device.
    #[sea_orm(string_value = "flash")]
    Flash,
}

/// Lifecycle status of a queued action.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_device_action_queue_status"
)]
pub enum QueueJobStatus {
    #[sea_orm(string_value = "queued")]
    Queued,
    #[sea_orm(string_value = "running")]
    Running,
    #[sea_orm(string_value = "waiting")]
    Waiting,
    /// Polling after a failure before retry/terminal transition.
    #[sea_orm(string_value = "failure_polling")]
    FailurePolling,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
}

/// How the dispatcher chose target devices for this job.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_device_action_queue_mode"
)]
pub enum QueueDispatchMode {
    /// Any free device of `device_type`.
    #[sea_orm(string_value = "deviceType")]
    DeviceType,
    /// Multiple devices in one submission.
    #[sea_orm(string_value = "bulk")]
    Bulk,
    /// Explicit `target_device_id`.
    #[sea_orm(string_value = "deviceSpecific")]
    DeviceSpecific,
}

/// One flash/test queue entry awaiting or undergoing worker processing.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_action_queue", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Linked test execution business key when type is test.
    pub test_id: Option<String>,
    pub device_type: String,
    pub r#type: QueueJobType,
    pub release_id: Option<Uuid>,
    pub version: Option<String>,
    pub target_device_id: Option<String>,
    /// Nullable in `tables.sql` (default `queued` when omitted on insert).
    pub status: Option<QueueJobStatus>,
    pub user_id: String,
    pub mode: QueueDispatchMode,
    /// Operator confirmed IPL flash step when required by Gen5 flow.
    pub ipl_flash_confirm: Option<bool>,
    /// Whether the IPL flash request was already sent to the controller.
    pub ipl_flash_request_sent: Option<bool>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

/// Foreign keys used when joining queue rows to devices, executions, and releases.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::device::Entity",
        from = "Column::TargetDeviceId",
        to = "super::device::Column::DeviceId"
    )]
    Device,
    #[sea_orm(
        belongs_to = "super::test_execution::Entity",
        from = "Column::TestId",
        to = "super::test_execution::Column::TestId"
    )]
    TestExecution,

    #[sea_orm(
        belongs_to = "super::release::Entity",
        from = "Column::ReleaseId",
        to = "super::release::Column::Id"
    )]
    Release,
}

impl Related<super::device::Entity> for Entity {
    /// PURPOSE: Join path to the targeted device row.
    /// USE: Load device context when dispatching or displaying a queue job.
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}

impl Related<super::test_execution::Entity> for Entity {
    /// PURPOSE: Join path to the owning test execution.
    /// USE: Correlate queue progress with execution phase updates.
    fn to() -> RelationDef {
        Relation::TestExecution.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

impl Related<super::release::Entity> for Entity {
    /// PURPOSE: Join path to the release being flashed.
    /// USE: Resolve artifact paths and version metadata for flash jobs.
    fn to() -> RelationDef {
        Relation::Release.def()
    }
}
