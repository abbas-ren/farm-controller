use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_device_action_queue_type"
)]
pub enum QueueJobType {
    #[sea_orm(string_value = "test")]
    Test,
    #[sea_orm(string_value = "flash")]
    Flash,
}
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
    #[sea_orm(string_value = "failure_polling")]
    FailurePolling,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_device_action_queue_mode"
)]
pub enum QueueDispatchMode {
    #[sea_orm(string_value = "deviceType")]
    DeviceType,
    #[sea_orm(string_value = "bulk")]
    Bulk,
    #[sea_orm(string_value = "deviceSpecific")]
    DeviceSpecific,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_action_queue", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub test_id: Option<String>,
    pub device_type: String,
    pub r#type: QueueJobType,
    pub release_id: Option<Uuid>,
    pub version: Option<String>,
    pub target_device_id: Option<String>,
    pub status: QueueJobStatus,
    pub user_id: String,
    pub mode: QueueDispatchMode,
    pub ipl_flash_confirm: Option<bool>,
    pub ipl_flash_request_sent: Option<bool>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

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
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}
impl Related<super::test_execution::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::TestExecution.def()
    }
}
impl ActiveModelBehavior for ActiveModel {}
impl Related<super::release::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Release.def()
    }
}
