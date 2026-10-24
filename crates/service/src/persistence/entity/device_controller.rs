//! Device controller registry (`device_controllers` table).
//!
//! Ports `farm_controller_old/device-service/src/models/DeviceController.ts`.
//! Controllers are edge hosts that manage relays, UART mappings, and DUT
//! connectivity; heartbeats and metrics hang off these identifiers.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Administrative approval state for a controller host.
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

/// Liveness view derived from heartbeat traffic.
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

/// One edge controller that owns relays and attached devices.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_controllers", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub device_controller_id: String,

    pub mac_address: String,
    pub ip_address: String,
    pub name: Option<String>,
    pub device_family: Option<String>,
    /// JSON mapping of ports / topology reported by the controller agent.
    pub mappings: Json,

    pub status: DeviceControllerStatus,
    pub state: DeviceControllerState,

    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,

    pub deleted_at: Option<DateTimeUtc>,
}

/// Controllers own many relays used for power control.
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
    /// PURPOSE: Join path from controller to its USB/serial relays.
    /// USE: Expand relay inventory when configuring power channels.
    fn to() -> RelationDef {
        Relation::Relays.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
