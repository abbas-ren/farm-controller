//! Database connectivity, SeaORM entities, and retention cleanup.
//!
//! This module replaces the Sequelize pool and model layer from
//! `farm_controller_old/device-service` with a shared `sqlx` [`PgPool`] plus a
//! SeaORM view over the same connections. Application services and workers obtain
//! [`Database`] from `AppState` for queries, migrations, readiness probes, and
//! batched retention deletes of high-volume tables.

pub mod entity;
pub mod pg_enum;
pub mod projection;
#[cfg(test)]
pub mod tests;

use std::{
    str::FromStr,
    time::{Duration, Instant},
};

use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
    sea_query::{Expr, LockBehavior, LockType},
};
use serde::Deserialize;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use crate::{config::DatabaseConfig, error::AppError, observability::Metrics};

use self::entity::{
    alert, device_action_queue, device_controller_heartbeat, device_controller_metrics,
    gen5_mapping_queue, log_entry, retention_policy,
};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Shared Postgres access for FarmController.
///
/// Holds one `sqlx` pool and a SeaORM connector derived from it so ORM and raw
/// SQL never open separate pools.
#[derive(Clone)]
pub struct Database {
    // SeaORM view over the same PgPool — avoids a second connection pool.
    orm: sea_orm::DatabaseConnection,
    pool: PgPool,
    metrics: Metrics,
}

impl Database {
    /// PURPOSE: Open the Postgres pool, wrap it in SeaORM, optionally migrate.
    /// USE: Called once during `AppState::bootstrap`. Records connect/migrate
    /// metrics. Side effect: may apply embedded sqlx migrations when configured.
    pub async fn connect(config: &DatabaseConfig, metrics: Metrics) -> Result<Self, AppError> {
        let url = config
            .url
            .as_deref()
            .ok_or_else(|| AppError::Configuration("database.url is required".to_owned()))?;
        let options = PgConnectOptions::from_str(url)
            .map_err(|error| AppError::Configuration(format!("invalid database URL: {error}")))?
            .application_name("farmcontroller");

        let started = Instant::now();
        let pool = PgPoolOptions::new()
            .min_connections(config.min_connections)
            .max_connections(config.max_connections)
            .acquire_timeout(Duration::from_secs(config.acquire_timeout_seconds))
            .idle_timeout(Duration::from_secs(config.idle_timeout_seconds))
            .max_lifetime(Duration::from_secs(config.max_lifetime_seconds))
            .connect_with(options)
            .await?;
        // Same PgPool — no second connection pool
        let orm = sea_orm::SqlxPostgresConnector::from_sqlx_postgres_pool(pool.clone());

        metrics.record_database_operation("connect", "success", started.elapsed());

        if config.run_migrations {
            let started = Instant::now();
            match MIGRATOR.run(&pool).await {
                Ok(()) => {
                    metrics.record_database_operation("migrate", "success", started.elapsed())
                }
                Err(error) => {
                    metrics.record_database_operation("migrate", "failure", started.elapsed());
                    return Err(error.into());
                }
            }
        }

        Ok(Self { orm, pool, metrics })
    }

    /// PURPOSE: Cheap readiness check against Postgres via SeaORM ping.
    /// USE: Health endpoints invoke this; records success/failure latency metrics.
    pub async fn ping(&self) -> Result<(), sea_orm::DbErr> {
        let started = Instant::now();
        let result = self.orm().ping().await;
        self.metrics.record_database_operation(
            "readiness",
            if result.is_ok() { "success" } else { "failure" },
            started.elapsed(),
        );
        result
    }

    /// PURPOSE: Expose the underlying sqlx pool for raw SQL or migrations.
    /// USE: Prefer SeaORM via [`Self::orm`] for entity work; use the pool when
    /// a crate needs sqlx-specific APIs.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// PURPOSE: Expose the SeaORM connection for entity queries and transactions.
    /// USE: All typed persistence helpers and workers query through this handle.
    pub fn orm(&self) -> &sea_orm::DatabaseConnection {
        &self.orm
    }

    /// PURPOSE: Load enabled retention policy rows from `farmcontroller.retention_policies`.
    /// USE: Retention workers call this each cycle to decide which tables to clean.
    /// Maps ORM models into the lightweight [`RetentionPolicy`] DTO.
    pub async fn enabled_retention_policies(
        &self,
    ) -> Result<Vec<RetentionPolicy>, sea_orm::DbErr> {
        Ok(retention_policy::Entity::find()
            .filter(retention_policy::Column::Enabled.eq(true))
            .order_by_asc(retention_policy::Column::TableName)
            .all(self.orm())
            .await?
            .into_iter()
            .map(|row| RetentionPolicy {
                table_name: row.table_name,
                timestamp_column: row.timestamp_column,
                retention_days: row.retention_days,
                batch_size: row.batch_size,
                archive_before_delete: row.archive_before_delete,
            })
            .collect())
    }

    /// PURPOSE: Delete one aged batch for a retention policy.
    /// USE: Retention worker loops this until a batch returns zero rows.
    /// Side effects: begins a transaction, locks candidate rows with
    /// `FOR UPDATE SKIP LOCKED`, deletes them, commits. Refuses policies that
    /// require archival first.
    pub async fn clean_retention_batch(
        &self,
        policy: &RetentionPolicy,
    ) -> Result<u64, RetentionError> {
        if policy.archive_before_delete {
            return Err(RetentionError::ArchiveRequired(policy.table_name.clone()));
        }
        let target = RetentionTarget::from_policy(policy)?;
        let started = Instant::now();
        let result = target
            .delete_batch(self.orm(), policy.retention_days, policy.batch_size)
            .await;
        self.metrics.record_database_operation(
            "retention_cleanup",
            if result.is_ok() { "success" } else { "failure" },
            started.elapsed(),
        );
        Ok(result?)
    }
}

/// Runtime DTO for a retention rule (not the SeaORM entity).
#[derive(Debug, Deserialize)]
pub struct RetentionPolicy {
    pub table_name: String,
    pub timestamp_column: String,
    pub retention_days: i32,
    pub batch_size: i32,
    pub archive_before_delete: bool,
}

/// Errors from retention target resolution or delete batches.
#[derive(Debug, thiserror::Error)]
pub enum RetentionError {
    /// Table/column pair is not in the hard-coded allow-list (SQL injection guard).
    #[error("unsupported retention target: {0}.{1}")]
    UnsupportedTarget(String, String),
    /// Policy requires archival that this binary does not yet perform.
    #[error("retention target requires archival before deletion: {0}")]
    ArchiveRequired(String),
    #[error(transparent)]
    Database(#[from] sea_orm::DbErr),
}

/// Allow-listed high-volume tables eligible for automated cleanup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetentionTarget {
    ControllerHeartbeats,
    ControllerMetrics,
    LogEntries,
    DeviceActionQueue,
    Gen5MappingQueue,
    Alerts,
}

impl RetentionTarget {
    /// PURPOSE: Map a policy's table/column names onto a typed cleanup target.
    /// USE: Called before every batch delete. Rejects unknown identifiers so
    /// retention never interpolates arbitrary SQL table names.
    fn from_policy(policy: &RetentionPolicy) -> Result<Self, RetentionError> {
        match (policy.table_name.as_str(), policy.timestamp_column.as_str()) {
            ("device_controller_heartbeats", "timestamp") => Ok(Self::ControllerHeartbeats),
            ("device_controller_metrics", "collectedAt") => Ok(Self::ControllerMetrics),
            ("log_entries", "timestamp") => Ok(Self::LogEntries),
            ("device_action_queue", "updatedAt") => Ok(Self::DeviceActionQueue),
            ("gen5_mapping_queue", "updatedAt") => Ok(Self::Gen5MappingQueue),
            // Alerts keep legacy snake_case created_at (unlike camelCase peers).
            ("alerts", "created_at") => Ok(Self::Alerts),
            _ => Err(RetentionError::UnsupportedTarget(
                policy.table_name.clone(),
                policy.timestamp_column.clone(),
            )),
        }
    }

    /// PURPOSE: Delete up to `batch_size` aged rows for this target in one transaction.
    /// USE: Invoked by [`Database::clean_retention_batch`]. Uses age filter
    /// `timestamp < now() - retention_days`, optional terminal-status filters for
    /// queues/alerts, and `FOR UPDATE SKIP LOCKED` so concurrent cleaners do not
    /// block each other.
    async fn delete_batch(
        self,
        orm: &sea_orm::DatabaseConnection,
        retention_days: i32,
        batch_size: i32,
    ) -> Result<u64, sea_orm::DbErr> {
        let transaction = orm.begin().await?;
        let limit = u64::try_from(batch_size.max(0)).unwrap_or(0);
        let days = retention_days as i64;
        let deleted = match self {
            Self::ControllerHeartbeats => {
                let ids = device_controller_heartbeat::Entity::find()
                    .filter(Expr::cust_with_values(
                        r#"timestamp < now() - ($1 * interval '1 day')"#,
                        [days],
                    ))
                    .order_by_asc(device_controller_heartbeat::Column::Timestamp)
                    .limit(limit)
                    // Skip rows locked by another cleaner/worker.
                    .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
                    .select_only()
                    .column(device_controller_heartbeat::Column::Id)
                    .into_tuple::<i32>()
                    .all(&transaction)
                    .await?;
                if ids.is_empty() {
                    0
                } else {
                    device_controller_heartbeat::Entity::delete_many()
                        .filter(device_controller_heartbeat::Column::Id.is_in(ids))
                        .exec(&transaction)
                        .await?
                        .rows_affected
                }
            }
            Self::ControllerMetrics => {
                let ids = device_controller_metrics::Entity::find()
                    .filter(Expr::cust_with_values(
                        r#""collectedAt" < now() - ($1 * interval '1 day')"#,
                        [days],
                    ))
                    .order_by_asc(device_controller_metrics::Column::CollectedAt)
                    .limit(limit)
                    .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
                    .select_only()
                    .column(device_controller_metrics::Column::Id)
                    .into_tuple::<i32>()
                    .all(&transaction)
                    .await?;
                if ids.is_empty() {
                    0
                } else {
                    device_controller_metrics::Entity::delete_many()
                        .filter(device_controller_metrics::Column::Id.is_in(ids))
                        .exec(&transaction)
                        .await?
                        .rows_affected
                }
            }
            Self::LogEntries => {
                let ids = log_entry::Entity::find()
                    .filter(Expr::cust_with_values(
                        r#"timestamp < now() - ($1 * interval '1 day')"#,
                        [days],
                    ))
                    .order_by_asc(log_entry::Column::Timestamp)
                    .limit(limit)
                    .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
                    .select_only()
                    .column(log_entry::Column::Id)
                    .into_tuple::<i32>()
                    .all(&transaction)
                    .await?;
                if ids.is_empty() {
                    0
                } else {
                    log_entry::Entity::delete_many()
                        .filter(log_entry::Column::Id.is_in(ids))
                        .exec(&transaction)
                        .await?
                        .rows_affected
                }
            }
            Self::DeviceActionQueue => {
                // Only terminal jobs — never delete in-flight queue work.
                let ids = device_action_queue::Entity::find()
                    .filter(Expr::cust_with_values(
                        r#""updatedAt" < now() - ($1 * interval '1 day')"#,
                        [days],
                    ))
                    .filter(device_action_queue::Column::Status.is_in([
                        device_action_queue::QueueJobStatus::Completed,
                        device_action_queue::QueueJobStatus::Cancelled,
                    ]))
                    .order_by_asc(device_action_queue::Column::UpdatedAt)
                    .limit(limit)
                    .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
                    .select_only()
                    .column(device_action_queue::Column::Id)
                    .into_tuple::<uuid::Uuid>()
                    .all(&transaction)
                    .await?;
                if ids.is_empty() {
                    0
                } else {
                    device_action_queue::Entity::delete_many()
                        .filter(device_action_queue::Column::Id.is_in(ids))
                        .exec(&transaction)
                        .await?
                        .rows_affected
                }
            }
            Self::Gen5MappingQueue => {
                // Same terminal-status gate as device_action_queue retention.
                let ids = gen5_mapping_queue::Entity::find()
                    .filter(Expr::cust_with_values(
                        r#""updatedAt" < now() - ($1 * interval '1 day')"#,
                        [days],
                    ))
                    .filter(gen5_mapping_queue::Column::Status.is_in([
                        gen5_mapping_queue::QueueJobStatus::Completed,
                        gen5_mapping_queue::QueueJobStatus::Cancelled,
                    ]))
                    .order_by_asc(gen5_mapping_queue::Column::UpdatedAt)
                    .limit(limit)
                    .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
                    .select_only()
                    .column(gen5_mapping_queue::Column::Id)
                    .into_tuple::<uuid::Uuid>()
                    .all(&transaction)
                    .await?;
                if ids.is_empty() {
                    0
                } else {
                    gen5_mapping_queue::Entity::delete_many()
                        .filter(gen5_mapping_queue::Column::Id.is_in(ids))
                        .exec(&transaction)
                        .await?
                        .rows_affected
                }
            }
            Self::Alerts => {
                // Only acknowledged alerts; unread must remain for the UI.
                let ids = alert::Entity::find()
                    .filter(Expr::cust_with_values(
                        r#"created_at < now() - ($1 * interval '1 day')"#,
                        [days],
                    ))
                    .filter(alert::Column::Status.eq(alert::AlertStatus::Read))
                    .order_by_asc(alert::Column::CreatedAt)
                    .limit(limit)
                    .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
                    .select_only()
                    .column(alert::Column::Id)
                    .into_tuple::<uuid::Uuid>()
                    .all(&transaction)
                    .await?;
                if ids.is_empty() {
                    0
                } else {
                    alert::Entity::delete_many()
                        .filter(alert::Column::Id.is_in(ids))
                        .exec(&transaction)
                        .await?
                        .rows_affected
                }
            }
        };
        transaction.commit().await?;
        Ok(deleted)
    }
}
