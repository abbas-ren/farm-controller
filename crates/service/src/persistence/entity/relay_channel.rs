use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_relay_channels_state"
)]
pub enum ChannelState {
    #[sea_orm(string_value = "on")]
    On,
    #[sea_orm(string_value = "off")]
    Off,
}
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "relay_channels", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub relay_id: Uuid,
    pub channel_number: i32,
    pub device_id: Option<String>,
    pub state: Option<ChannelState>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub deleted_at: Option<DateTimeUtc>,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::relay::Entity",
        from = "Column::RelayId",
        to = "super::relay::Column::Id"
    )]
    Relay,
    #[sea_orm(
        belongs_to = "super::device::Entity",
        from = "Column::DeviceId",
        to = "super::device::Column::DeviceId"
    )]
    Device,
}
impl Related<super::relay::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Relay.def()
    }
}
impl Related<super::device::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}
impl ActiveModelBehavior for ActiveModel {}
