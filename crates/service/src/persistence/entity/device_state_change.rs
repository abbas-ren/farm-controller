use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_device_state_change_state"
)]
pub enum DeviceStateChangeState {
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

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_state_change", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub device_id: String,
    pub state: DeviceStateChangeState,
    pub changed_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::device::Entity",
        from = "Column::DeviceId",
        to = "super::device::Column::DeviceId"
    )]
    Device,
}
impl Related<super::device::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}
impl ActiveModelBehavior for ActiveModel {}
