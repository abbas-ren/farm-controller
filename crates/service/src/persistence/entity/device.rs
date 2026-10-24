use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "enum_devices_state")]
pub enum DeviceState {
    #[sea_orm(string_value = "busy")]
    Busy,
    #[sea_orm(string_value = "not_reachable")]
    NotReachable,
    #[sea_orm(string_value = "free")]
    Free,
    #[sea_orm(string_value = "faulty")]
    Faulty,
    #[sea_orm(string_value = "unknown")]
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_devices_status"
)]
pub enum DeviceStatus {
    #[sea_orm(string_value = "approved")]
    Approved,
    #[sea_orm(string_value = "requested")]
    Requested,
    #[sea_orm(string_value = "declined")]
    Declined,
}
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_devices_lastExecutionStatus"
)]
pub enum TestStatus {
    #[sea_orm(string_value = "not_executed")]
    NotExecuted,
    #[sea_orm(string_value = "in_progress")]
    InProgress,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "queued")]
    Queued,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
    #[sea_orm(string_value = "failed")]
    Failed,
}
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "enum_devices_power")]
pub enum DevicePower {
    #[sea_orm(string_value = "on")]
    On,
    #[sea_orm(string_value = "off")]
    Off,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "devices", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub device_name: Option<String>,
    pub device_family: Option<String>,
    #[sea_orm(unique)]
    pub device_id: String,
    pub mac_address: Option<String>,
    pub ip_address: Option<String>,
    pub device_type: Option<String>,
    pub state: Option<DeviceState>,
    pub status: Option<DeviceStatus>,
    pub last_connected_on: DateTimeUtc,
    pub controller_id: Option<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub status_action_by: Option<String>,
    pub last_test_execution: Option<String>,
    pub last_execution_status: Option<TestStatus>,
    pub state_updated_at: Option<DateTimeUtc>,
    pub build_id: Option<String>,
    pub heartbeat_timer: i32,
    pub upgrading: Option<bool>,
    pub flashing: Option<bool>,
    pub software_version: Option<String>,
    pub power: Option<DevicePower>,
    pub nfs_path: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub deleted_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        has_many = "super::device_interface::Entity",
        from = "Column::DeviceId",
        to = "super::device_interface::Column::DeviceId"
    )]
    Interfaces,
    #[sea_orm(
        has_many = "super::test_execution::Entity",
        from = "Column::DeviceId",
        to = "super::test_execution::Column::DeviceId"
    )]
    TestExecutions,
    #[sea_orm(
        has_many = "super::device_action_queue::Entity",
        from = "Column::DeviceId",
        to = "super::device_action_queue::Column::TargetDeviceId"
    )]
    DeviceActionQueues,
}
impl Related<super::device_interface::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Interfaces.def()
    }
}
impl Related<super::test_execution::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::TestExecutions.def()
    }
}
impl Related<super::device_action_queue::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::DeviceActionQueues.def()
    }
}
impl ActiveModelBehavior for ActiveModel {}
