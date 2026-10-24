//! Per-device interface status (`device_interfaces` table).
//!
//! Ports `DeviceInterface.ts`. Tracks Ethernet, USB, UART, and similar links
//! reported for a DUT so the UI and health checks can show port state.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Normalized link state across heterogeneous interface types.
#[derive(
    Clone, Debug, Default, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_device_interfaces_status"
)]
pub enum DeviceInterfaceStatus {
    #[sea_orm(string_value = "up")]
    Up,
    #[sea_orm(string_value = "down")]
    Down,
    #[sea_orm(string_value = "plugged")]
    Plugged,
    #[sea_orm(string_value = "unplugged")]
    Unplugged,
    #[sea_orm(string_value = "connected")]
    Connected,
    #[sea_orm(string_value = "disconnected")]
    Disconnected,
    #[sea_orm(string_value = "unknown")]
    #[default]
    Unknown,
    #[sea_orm(string_value = "available")]
    Available,
    #[sea_orm(string_value = "not_available")]
    NotAvailable,
}

/// One interface attachment belonging to a device.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_interfaces", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    /// Stable interface identifier from the controller agent.
    pub interface_id: String,
    /// Coarse category (ethernet, usb, uart, …).
    pub interface_type: String,
    pub device_id: String,
    pub name: Option<String>,
    pub r#type: Option<String>,
    pub status: Option<DeviceInterfaceStatus>,
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
    /// PURPOSE: Join path to the owning device.
    /// USE: Load device context when listing or updating interfaces.
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
