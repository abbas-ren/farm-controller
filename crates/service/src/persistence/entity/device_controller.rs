use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_device_controllers_status"
)]
pub enum DeviceControllerStatus {
    #[sea_orm(string_value = "approved")]
    Approved,
    #[sea_orm(string_value = "requested")]
    Requested,
}
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_device_controllers_state"
)]
pub enum DeviceControllerState {
    #[sea_orm(string_value = "not-reachable")]
    NotReachable,
    #[sea_orm(string_value = "active")]
    Active,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_controllers", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub device_controller_id: String,

    pub mac_address: String,
    pub ip_address: String,
    pub name: Option<String>,
    pub device_family: Option<String>,
    pub mappings: Json,

    pub status: DeviceControllerStatus,
    pub state: DeviceControllerState,

    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,

    pub deleted_at: Option<DateTimeUtc>,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        has_many = "super::relay::Entity",
        from = "Column::DeviceControllerId",
        to = "super::relay::Column::DeviceControllerId"
    )]
    Relays,
}
impl Related<super::relay::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Relays.def()
    }
}
impl ActiveModelBehavior for ActiveModel {}
