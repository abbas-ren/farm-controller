//! Device persistence contracts composed from feature-sized capabilities.
//!
//! Handlers and workers use `DeviceRepository` as one object-safe facade, while
//! PostgreSQL and test implementations implement the smaller traits owned by
//! execution, build, inventory, controller, relay, analytics, and alert domains.

use super::{
    AlertList, AvailableRelayDevicesQuery, BuildFilters, BuildFlagRequest, BuildList,
    BuildListQuery, CallbackStatus, ControllerEditRequest, ControllerHeartbeat, ControllerList,
    ControllerListQuery, ControllerRegistration, DeviceAction, DeviceCsvQuery, DeviceList,
    DeviceListQuery, DeviceRegistration, DeviceStateAnalytics, DeviceStateDetailedAnalytics,
    DeviceTypeFolder, ExecutionList, ExecutionListQuery, LegacyRelayChannelListQuery,
    LegacyRelayConfiguration, LegacyRelayConflictQuery, LegacyRelayListQuery, LogCreateRequest,
    LogListQuery, RelayChannelConfiguration, RelayConfirmationResponse, RelayConflictState,
    UserDeviceListQuery,
    error::DeviceRepositoryError,
    repository_types::{
        BuildDeleteTarget, BuildFlagResult, BuildUploadFinalization, BuildUploadResult,
        ControllerDeleteTarget, ControllerHeartbeatResult, DeviceActionTarget, DeviceDeleteTarget,
        DeviceFlashingResult, DeviceHeartbeatResult, DevicePowerTarget, DeviceRebootTarget,
        DeviceRegistrationResult, ExecutionCreation, FaultyReportCreate, FaultyReportCreation,
        FaultyReportFinalization, FlashConfirmationResult, RegistrationResult,
        RelayConfigurationResult, RelayIdentityTarget, RelayStateTarget, TestCancellationResult,
        TestCompletionTarget, TestExportRow,
    },
    types::TtyEntry,
};

mod alerts;
mod analytics;
mod builds;
mod controllers;
mod device_artifacts;
mod execution_lifecycle;
mod execution_queries;
mod execution_reports;
mod faulty_reports;
mod heartbeats;
mod inventory;
mod power_relays;
mod registration;
mod user_devices;

pub(crate) use alerts::AlertRepository;
pub(crate) use analytics::AnalyticsRepository;
pub(crate) use builds::BuildRepository;
pub(crate) use controllers::ControllerRepository;
pub(crate) use device_artifacts::DeviceArtifactRepository;
pub(crate) use execution_lifecycle::ExecutionLifecycleRepository;
pub(crate) use execution_queries::ExecutionQueryRepository;
pub(crate) use execution_reports::ExecutionReportRepository;
pub(crate) use faulty_reports::FaultyReportRepository;
pub(crate) use heartbeats::HeartbeatRepository;
pub(crate) use inventory::DeviceInventoryRepository;
pub(crate) use power_relays::PowerRelayRepository;
pub(crate) use registration::DeviceRegistrationRepository;
pub(crate) use user_devices::UserDeviceRepository;

/// Composes the device persistence capabilities used by handlers and workers.
pub(crate) trait DeviceRepository:
    ExecutionLifecycleRepository
    + ExecutionQueryRepository
    + ExecutionReportRepository
    + BuildRepository
    + DeviceRegistrationRepository
    + DeviceInventoryRepository
    + ControllerRepository
    + UserDeviceRepository
    + DeviceArtifactRepository
    + PowerRelayRepository
    + AnalyticsRepository
    + AlertRepository
    + FaultyReportRepository
    + HeartbeatRepository
    + Send
    + Sync
{
}

impl<T> DeviceRepository for T where
    T: ExecutionLifecycleRepository
        + ExecutionQueryRepository
        + ExecutionReportRepository
        + BuildRepository
        + DeviceRegistrationRepository
        + DeviceInventoryRepository
        + ControllerRepository
        + UserDeviceRepository
        + DeviceArtifactRepository
        + PowerRelayRepository
        + AnalyticsRepository
        + AlertRepository
        + FaultyReportRepository
        + HeartbeatRepository
        + Send
        + Sync
{
}
