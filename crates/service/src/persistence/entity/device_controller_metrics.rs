//! Controller resource metrics (`device_controller_metrics` table).
//!
//! Ports `DeviceControllerMetrics.ts` / `SystemMetricsService`. Stores periodic
//! CPU/memory/disk/network snapshots for observability dashboards. Retention
//! ages rows by `"collectedAt"`.

use sea_orm::entity::prelude::*;

/// One resource snapshot for a device controller.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_controller_metrics", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub controller_id: String,
    pub cpu_current: String,
    pub cpu_total: String,
    pub cpu_usage_percent: f64,
    pub memory_used: String,
    pub memory_total: String,
    pub memory_usage_percent: f64,
    pub network_upload: String,
    pub network_download: String,
    pub disk_used: String,
    pub disk_total: String,
    pub disk_usage_percent: f64,
    /// Collection time; retention filter uses this camelCase column.
    pub collected_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
