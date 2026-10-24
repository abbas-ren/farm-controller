//! Controller heartbeat samples (`device_controller_heartbeats` table).
//!
//! Ports `DeviceControllerHeartbeat.ts` / `ControllerHeartbeatQueueService`.
//! High-volume time-series data; subject to retention batch deletes ordered by
//! `timestamp` with `FOR UPDATE SKIP LOCKED`.

use sea_orm::entity::prelude::*;

/// One heartbeat sample received from a device controller agent.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_controller_heartbeats", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    /// Matches `device_controllers.deviceControllerId`.
    pub controller_id: String,
    /// Sample time used by retention age filters.
    pub timestamp: DateTimeUtc,
    /// Declared timeout seconds from the agent payload.
    pub timeout: i32,
    /// Raw heartbeat JSON (health details, counters, etc.).
    pub data: Json,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
