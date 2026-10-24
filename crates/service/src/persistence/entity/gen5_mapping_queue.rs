//! Gen5 UART mapping queue (`gen5_mapping_queue` table).
//!
//! Ports `Gen5MappingQueue.ts` / `Gen5MappingQueueService`. Jobs drive automated
//! discovery and verification of Gen5 serial mappings, including retry/backoff
//! fields. Retention deletes terminal `completed` / `cancelled` rows only.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Lifecycle status for a Gen5 mapping job (mirrors device-action queue statuses).
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_gen5_mapping_queue_status"
)]
pub enum QueueJobStatus {
    #[sea_orm(string_value = "queued")]
    Queued,
    #[sea_orm(string_value = "running")]
    Running,
    #[sea_orm(string_value = "waiting")]
    Waiting,
    #[sea_orm(string_value = "failure_polling")]
    FailurePolling,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
}

/// One queued Gen5 mapping attempt for a controller MAC.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "gen5_mapping_queue", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Controller MAC used to locate the mapping target.
    pub mac_address: String,
    pub user_id: String,
    pub status: QueueJobStatus,
    pub retry_count: i32,
    pub last_error: Option<String>,
    /// Device-controller IP observed at last failure (for diagnostics).
    pub failure_dc_ip: Option<String>,
    /// Mapping count before the attempt, used to detect progress.
    pub baseline_mapping_count: Option<i32>,
    pub next_retry_at: Option<DateTimeUtc>,
    /// Index into the configured retry delay schedule.
    pub retry_schedule_index: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
