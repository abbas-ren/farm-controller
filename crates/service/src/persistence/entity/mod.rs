//! SeaORM entity modules mirroring device-service Sequelize models.
//!
//! Each submodule maps one PostgreSQL table used by FarmController. Column names
//! keep the legacy camelCase (or snake_case for `alerts`) shape so the Rust
//! service can share the same schema as `farm_controller_old/device-service`.
//! Callers import entities through this module when building queries, ActiveModels,
//! or relation joins in services and workers.

pub mod alert;
pub mod device;
pub mod device_action_queue;
pub mod device_controller;
pub mod device_controller_heartbeat;
pub mod device_controller_metrics;
pub mod device_interface;
pub mod device_state_change;
pub mod device_type_folder;
pub mod device_uart_mapping;
pub mod execution_report;
pub mod fallback_update;
pub mod faulty_report;
pub mod gen5_mapping_queue;
pub mod gen5_reboot_attempt;
pub mod heartbeat;
pub mod log_entry;
pub mod pending_relay_config;
pub mod relay;
pub mod relay_channel;
pub mod release;
pub mod retention_policy;
pub mod test_case;
pub mod test_execution;
pub mod upload_build;

pub mod common;
