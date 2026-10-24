//! Structured application log entries (`log_entries` table).
//!
//! Ports `LogEntry.ts` / `LogService`. Stores keyed log blobs for builds, tests,
//! devices, or general events. Retention ages rows by `timestamp`.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Domain bucket used when querying logs by reference.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_log_entries_type"
)]
pub enum LogEntryType {
    #[sea_orm(string_value = "build")]
    Build,
    #[sea_orm(string_value = "test")]
    Test,
    #[sea_orm(string_value = "device")]
    Device,
    #[sea_orm(string_value = "general")]
    General,
}

/// Severity of a stored log entry.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_log_entries_level"
)]
pub enum LogEntryLevel {
    #[sea_orm(string_value = "info")]
    Info,
    #[sea_orm(string_value = "warn")]
    Warn,
    #[sea_orm(string_value = "error")]
    Error,
    #[sea_orm(string_value = "debug")]
    Debug,
}

/// One persisted log record keyed by type + reference id.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "log_entries", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub r#type: LogEntryType,
    /// Build id, test id, device id, etc. depending on [`LogEntryType`].
    pub reference_id: String,
    /// Structured message / payload JSON.
    pub data: Json,
    pub level: LogEntryLevel,
    /// Event time used by retention cleanup.
    pub timestamp: DateTimeUtc,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
