//! Read-model projections assembled from SeaORM / raw SQL joins.
//!
//! Unlike `entity` modules (1:1 table maps), projections describe denormalized
//! row shapes used by queue dispatchers and APIs. They replace Sequelize
//! `include` / raw query result DTOs from device-service workers.

pub mod device_action_queue;

pub use device_action_queue::*;
