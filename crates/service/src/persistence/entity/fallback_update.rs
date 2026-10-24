//! Fallback flash jobs (`fallback_updates` table).
//!
//! Ports `FallbackUpdate.ts`. When a primary flash/test path fails, workers may
//! schedule a fallback image flash tracked by these rows.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Progress of a fallback flash attempt.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_fallback_updates_status"
)]
pub enum FallbackStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "flashing")]
    Flashing,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "failed")]
    Failed,
}

/// One fallback flash request tied to a device and originating test.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "fallback_updates", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub device_id: String,
    pub release_id: String,
    pub test_id: String,
    /// Filesystem path to the fallback image/artifacts.
    pub fallback_path: String,
    pub status: FallbackStatus,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
