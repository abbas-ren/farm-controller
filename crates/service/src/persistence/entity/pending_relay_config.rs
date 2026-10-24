//! In-flight relay channel configuration (`pending_relay_configs` table).
//!
//! Ports `PendingRelayConfig.ts` / `RelayConfigTimeoutService`. Tracks a
//! configure-and-confirm handshake between FarmController and a device
//! controller when assigning a relay channel to a device MAC.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Handshake status for a pending relay configuration.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_pending_relay_configs_status"
)]
pub enum PendingRelayConfigStatus {
    /// Awaiting controller confirmation.
    #[sea_orm(string_value = "pending")]
    Pending,
    /// Controller acknowledged the mapping.
    #[sea_orm(string_value = "confirmed")]
    Confirmed,
    /// Timed out or rejected.
    #[sea_orm(string_value = "failed")]
    Failed,
}

/// One outstanding relay-channel configuration request.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "pending_relay_configs", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub relay_channel_id: Uuid,
    pub relay_serial: String,
    pub channel_no: i32,
    pub device_mac: String,
    /// Controller address the config was sent to.
    pub controller_address: String,
    pub device_gen: Option<String>,
    pub status: PendingRelayConfigStatus,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
