//! PostgreSQL implementations of the device repository capabilities.
//!
//! Each child module implements one capability trait directly on
//! `PostgresDeviceRepository`; this module only owns the SeaORM adapter
//! construction, so persistence features remain independently navigable.
//! Replaces the Sequelize models/repositories behind Node DeviceService,
//! TestService, RelayService, BuildService, and AnalyticsService.

use std::collections::BTreeMap;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::repository::{
    AlertRepository, AnalyticsRepository, BuildRepository, ControllerRepository,
    DeviceArtifactRepository, DeviceInventoryRepository, DeviceRegistrationRepository,
    ExecutionLifecycleRepository, ExecutionQueryRepository, ExecutionReportRepository,
    FaultyReportRepository, HeartbeatRepository, PowerRelayRepository, UserDeviceRepository,
};
use super::*;
use super::{error::DeviceRepositoryError, repository_types::*, types::TtyEntry};

/// SeaORM-backed `DeviceRepository` shared by device HTTP handlers and workers.
pub(crate) struct PostgresDeviceRepository {
    /// SeaORM connection wrapping the shared sqlx pool.
    orm: sea_orm::DatabaseConnection,
}

impl PostgresDeviceRepository {
    /// PURPOSE: Wrap a sqlx `PgPool` as a SeaORM `DatabaseConnection`.
    /// USE: App startup when wiring `AppState.devices` / repository.
    pub(crate) fn new(pool: PgPool) -> Self {
        let orm = sea_orm::SqlxPostgresConnector::from_sqlx_postgres_pool(pool);
        Self { orm }
    }

    /// PURPOSE: Expose the SeaORM connection to capability impl modules and stores.
    /// USE: Every `PostgresDeviceRepository` trait method that delegates to a store.
    pub(crate) fn orm(&self) -> &sea_orm::DatabaseConnection {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::mod", "orm");
        &self.orm
    }
}

mod analytics;
mod builds;
mod controllers;
mod execution_lifecycle;
mod execution_queries;
mod execution_reports;
mod faulty_reports;
mod inventory;
pub(crate) mod json_models;
mod power_relays;
mod registration;
mod user_devices;
