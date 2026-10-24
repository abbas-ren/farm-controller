//! Shared DTOs and helpers for background workers.
//!
//! Types here are the claim/query shapes and transition payloads that workers
//! exchange with Postgres and the socket event hub. Runtime entrypoints live in
//! sibling worker modules; this package only carries data contracts.

mod common;
mod device_action;
mod device_action_watchdog;
mod device_controller;
mod device_heartbeat;
mod gen5_reboot_watchdog;
mod relay;
mod sys_metrics;
mod test_prep;
mod worker_manager;

pub use common::*;
pub use device_action::*;
pub use device_action_watchdog::*;
pub use device_controller::*;
pub use device_heartbeat::*;
pub use gen5_reboot_watchdog::*;
pub use relay::*;
pub use sys_metrics::*;
pub use test_prep::*;
pub use worker_manager::*;

/// Map a SeaORM [`DbErr`](sea_orm::DbErr) into an `sqlx::Error` for metrics/logging paths
/// that already speak sqlx.
///
/// Prefer this when a worker already records failures with sqlx-shaped errors
/// (legacy metric helpers) but queried via SeaORM.
pub fn err_seaorm_to_sqlx(err: sea_orm::DbErr) -> sqlx::Error {
    use sea_orm::{DbErr, RuntimeErr};

    match err {
        DbErr::Query(RuntimeErr::SqlxError(err))
        | DbErr::Exec(RuntimeErr::SqlxError(err))
        | DbErr::Conn(RuntimeErr::SqlxError(err)) => match std::sync::Arc::try_unwrap(err) {
            Ok(err) => err,
            Err(err) => sqlx::Error::Protocol(err.to_string()),
        },

        other => sqlx::Error::Protocol(other.to_string()),
    }
}
