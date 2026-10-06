use super::*;

pub(super) mod alerts_repository;
pub(super) mod analytics_repository;
mod execution_repository;
mod fake_providers;
pub(super) mod faulty_report_repository;
pub(super) mod heartbeat_repository;
mod inventory_repository;
pub(super) mod power_relay_repository;

#[allow(unused_imports)]
pub(super) use self::{
    alerts_repository as alerts, analytics_repository as analytics,
    faulty_report_repository as faulty_reports, heartbeat_repository as heartbeat,
    power_relay_repository as power_relay,
};
