use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "enum_relays_state")]
pub enum RelayState {
    #[sea_orm(string_value = "connected")]
    Connected,
    #[sea_orm(string_value = "disconnected")]
    Disconnected,
}
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "relays", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub serial_number: String,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    pub device_controller_id: String,
    pub state: RelayState,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub deleted_at: Option<DateTimeUtc>,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::device_controller::Entity",
        from = "Column::DeviceControllerId",
        to = "super::device_controller::Column::DeviceControllerId"
    )]
    DeviceController,
    #[sea_orm(
        has_many = "super::relay_channel::Entity",
        from = "Column::Id",
        to = "super::relay_channel::Column::RelayId"
    )]
    RelayChannels,
}
impl Related<super::device_controller::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::DeviceController.def()
    }
}
impl Related<super::relay_channel::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RelayChannels.def()
    }
}
impl ActiveModelBehavior for ActiveModel {}
