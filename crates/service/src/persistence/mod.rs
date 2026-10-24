pub mod entity;
pub mod orm_util;
pub mod projection;
#[cfg(test)]
pub mod tests;

use std::{
    str::FromStr,
    time::{Duration, Instant},
};

use sea_orm::{ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder};
use serde::Deserialize;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use crate::{config::DatabaseConfig, error::AppError, observability::Metrics};

use self::entity::retention_policy;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[derive(Clone)]
pub struct Database {
    // SeaORM view over the same PgPool.
    orm: sea_orm::DatabaseConnection,
    pool: PgPool,
    metrics: Metrics,
}

impl Database {
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

    pub async fn ping(&self) -> Result<(), sea_orm::DbErr> {
        #[derive(Debug, FromQueryResult)]
        struct PingRow {
            #[allow(dead_code)]
            n: i32,
        }
        let started = Instant::now();
        let result = orm_util::query_one::<_, PingRow>(self.orm(), "SELECT 1 AS n", vec![])
            .await
            .and_then(|row| row.ok_or(sea_orm::DbErr::RecordNotFound("ping".into())))
            .map(|_| ());
        self.metrics.record_database_operation(
            "readiness",
            if result.is_ok() { "success" } else { "failure" },
            started.elapsed(),
        );
        result
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn orm(&self) -> &sea_orm::DatabaseConnection {
        &self.orm
    }

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

    pub async fn clean_retention_batch(
        &self,
        policy: &RetentionPolicy,
    ) -> Result<u64, RetentionError> {
        if policy.archive_before_delete {
            return Err(RetentionError::ArchiveRequired(policy.table_name.clone()));
        }
        let target = RetentionTarget::from_policy(policy)?;
        let started = Instant::now();
        // Dynamic DELETE SQL is allowlisted via RetentionTarget — run through SeaORM Statement.
        let result = orm_util::execute(
            self.orm(),
            target.delete_sql(),
            vec![policy.retention_days.into(), policy.batch_size.into()],
        )
        .await;
        self.metrics.record_database_operation(
            "retention_cleanup",
            if result.is_ok() { "success" } else { "failure" },
            started.elapsed(),
        );
        Ok(result?)
    }
}

#[derive(Debug, Deserialize)]
pub struct RetentionPolicy {
    pub table_name: String,
    pub timestamp_column: String,
    pub retention_days: i32,
    pub batch_size: i32,
    pub archive_before_delete: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RetentionError {
    #[error("unsupported retention target: {0}.{1}")]
    UnsupportedTarget(String, String),
    #[error("retention target requires archival before deletion: {0}")]
    ArchiveRequired(String),
    #[error(transparent)]
    Database(#[from] sea_orm::DbErr),
}

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
    fn from_policy(policy: &RetentionPolicy) -> Result<Self, RetentionError> {
        match (policy.table_name.as_str(), policy.timestamp_column.as_str()) {
            ("device_controller_heartbeats", "timestamp") => Ok(Self::ControllerHeartbeats),
            ("device_controller_metrics", "collectedAt") => Ok(Self::ControllerMetrics),
            ("log_entries", "timestamp") => Ok(Self::LogEntries),
            ("device_action_queue", "updatedAt") => Ok(Self::DeviceActionQueue),
            ("gen5_mapping_queue", "updatedAt") => Ok(Self::Gen5MappingQueue),
            ("alerts", "created_at") => Ok(Self::Alerts),
            _ => Err(RetentionError::UnsupportedTarget(
                policy.table_name.clone(),
                policy.timestamp_column.clone(),
            )),
        }
    }

    fn delete_sql(self) -> &'static str {
        match self {
            Self::ControllerHeartbeats => {
                r#"
            WITH expired AS (
                SELECT ctid FROM device_controller_heartbeats
                WHERE timestamp < now() - ($1 * interval '1 day')
                ORDER BY timestamp LIMIT $2 FOR UPDATE SKIP LOCKED
            )
            DELETE FROM device_controller_heartbeats target
            USING expired WHERE target.ctid = expired.ctid
        "#
            }
            Self::ControllerMetrics => {
                r#"
            WITH expired AS (
                SELECT ctid FROM device_controller_metrics
                WHERE "collectedAt" < now() - ($1 * interval '1 day')
                ORDER BY "collectedAt" LIMIT $2 FOR UPDATE SKIP LOCKED
            )
            DELETE FROM device_controller_metrics target
            USING expired WHERE target.ctid = expired.ctid
        "#
            }
            Self::LogEntries => {
                r#"
            WITH expired AS (
                SELECT ctid FROM log_entries
                WHERE timestamp < now() - ($1 * interval '1 day')
                ORDER BY timestamp LIMIT $2 FOR UPDATE SKIP LOCKED
            )
            DELETE FROM log_entries target
            USING expired WHERE target.ctid = expired.ctid
        "#
            }
            Self::DeviceActionQueue => {
                r#"
            WITH expired AS (
                SELECT ctid FROM device_action_queue
                WHERE "updatedAt" < now() - ($1 * interval '1 day')
                                    AND status IN ('completed', 'cancelled')
                ORDER BY "updatedAt" LIMIT $2 FOR UPDATE SKIP LOCKED
            )
            DELETE FROM device_action_queue target
            USING expired WHERE target.ctid = expired.ctid
        "#
            }
            Self::Gen5MappingQueue => {
                r#"
            WITH expired AS (
                SELECT ctid FROM gen5_mapping_queue
                WHERE "updatedAt" < now() - ($1 * interval '1 day')
                                    AND status IN ('completed', 'cancelled')
                ORDER BY "updatedAt" LIMIT $2 FOR UPDATE SKIP LOCKED
            )
            DELETE FROM gen5_mapping_queue target
            USING expired WHERE target.ctid = expired.ctid
        "#
            }
            Self::Alerts => {
                r#"
            WITH expired AS (
                SELECT ctid FROM alerts
                WHERE created_at < now() - ($1 * interval '1 day')
                  AND status = 'read'
                ORDER BY created_at LIMIT $2 FOR UPDATE SKIP LOCKED
            )
            DELETE FROM alerts target
            USING expired WHERE target.ctid = expired.ctid
        "#
            }
        }
    }
}
