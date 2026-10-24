//! Background workers that drive queues, watchdogs, scanners, and metrics.
//!
//! Spawned by [`worker_manager::WorkerManager::start`] when the corresponding
//! config modules (`Workers`, `Device`, `Events`) are enabled. Each worker loops
//! on a poll interval (or mpsc channel), claims work under Postgres locking where
//! needed, and publishes farm WebSocket events / metrics as side effects.
//!
//! Legacy Node counterparts: `DeviceActionQueueService`, `HeartbeatQueueService`,
//! `Gen5MappingQueueService`, `Gen5RebootWatchdogService`,
//! `RelayConfigTimeoutService`, `ArtifactsScanner`, plus reports-service workers.

#[cfg(test)]
pub mod tests;

pub mod artifact_preparation;
pub mod artifact_scanner;
pub mod build_preparation;
pub mod device_action;
pub mod device_action_watchdog;
pub mod device_controller;
pub mod device_heartbeat;
pub mod fallback_flash;
pub mod gen5_mapping;
pub mod gen5_reboot_watchdog;
pub mod ipl_distribution;
pub mod relay_service;
pub mod retention;
pub mod system_metrics;
pub mod test_preconditions;
pub mod test_preparation;
pub mod worker_manager;

pub mod types;

pub(crate) use build_preparation::prepare_uploaded_build;
pub(crate) use ipl_distribution::{
    remove_build as remove_build_ipl, sync_controller as sync_controller_ipl,
};
