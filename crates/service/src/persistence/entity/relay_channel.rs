//! Individual power channels on a relay (`relay_channels` table).
//!
//! Ports `RelayChannel.ts`. Each channel may be bound to a DUT and toggled on/off
//! for power control during flash, reboot, or recovery flows.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Energized state of a relay channel.
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

/// One numbered channel on a parent relay.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "relay_channels", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub relay_id: Uuid,
    pub channel_number: i32,
    /// Bound DUT when configured; `None` if unassigned.
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
    /// PURPOSE: Join path to the parent relay hardware.
    /// USE: Load serial/VID/PID when issuing channel commands.
    fn to() -> RelationDef {
        Relation::Relay.def()
    }
}

impl Related<super::device::Entity> for Entity {
    /// PURPOSE: Join path to the DUT powered by this channel.
    /// USE: Resolve device identity when toggling power for a mapped channel.
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
