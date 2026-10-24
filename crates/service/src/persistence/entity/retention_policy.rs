//! Configurable table retention policies (`retention_policies` table).
//!
//! New to the Rust FarmController (not a Sequelize model in device-service).
//! Rows describe which high-volume tables the cleanup worker may delete from,
//! using an allow-listed `(table_name, timestamp_column)` pair enforced in
//! `persistence::RetentionTarget`.

use sea_orm::entity::prelude::*;

/// One enabled/disabled retention rule for a high-volume table.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "retention_policies", schema_name = "farmcontroller")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub table_name: String,
    /// Column used for age comparison (`timestamp`, `created_at`, …).
    pub timestamp_column: String,
    pub retention_days: i32,
    /// Max rows deleted per cleanup batch.
    pub batch_size: i32,
    pub enabled: bool,
    /// When true, cleanup refuses to delete until archival is implemented.
    pub archive_before_delete: bool,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
