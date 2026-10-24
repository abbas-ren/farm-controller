//! Device heartbeat samples (`heartbeats` table).
//!
//! Ports `HeartbeatModel.ts` / `HeartbeatQueueService`. Time-series liveness
//! data from DUTs (distinct from controller heartbeats).

use sea_orm::entity::prelude::*;

/// One heartbeat sample received from a device agent.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "heartbeats", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub device_id: String,
    pub timestamp: DateTimeUtc,
    /// Declared timeout seconds from the heartbeat payload.
    pub timeout: i32,
    /// Raw heartbeat JSON payload.
    pub data: Json,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
