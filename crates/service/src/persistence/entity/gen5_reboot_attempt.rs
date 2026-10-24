//! Gen5 reboot watchdog attempts (`gen5_reboot_attempts` table).
//!
//! Supports `Gen5RebootWatchdogService`-style flows: records the last power-cycle
//! attempt for a device so the watchdog can rate-limit or detect stuck remaps.
//! Lives in the `farmcontroller` schema.

use sea_orm::entity::prelude::*;

/// Last recorded Gen5 reboot attempt for a device (primary key = device_id).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "gen5_reboot_attempts", schema_name = "farmcontroller")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub device_id: String,
    pub attempted_at: DateTimeUtc,
    pub controller_ip: String,
    /// Relay / PDU port used for the power cycle.
    pub power_port: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
