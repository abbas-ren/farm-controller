//! Device HTTP compatibility, repository wiring, and public device-domain exports.
//! `routes` assembles sibling handler modules; repository methods remain behind the unchanged
//! repository trait, with Postgres behavior grouped by capability under `postgres`.
//! Store modules remain siblings and are invoked directly by those capability implementations.

#[cfg(test)]
use crate::state::AppState;
#[cfg(test)]
use async_trait::async_trait;
#[cfg(test)]
use axum::{Json, http::StatusCode};
#[cfg(test)]
use std::{collections::BTreeMap, sync::Arc};
#[cfg(test)]
use uuid::Uuid;

pub(crate) mod alert_handlers;
mod alert_store;
pub(crate) mod analytics_handlers;
mod analytics_store;
pub(crate) mod artifact_handlers;
mod artifacts;
pub(crate) mod build_handlers;
mod build_ingestion;
pub(crate) mod build_store;
mod callback_store;
mod constants;
mod controller_store;
mod csv_export;
pub(crate) mod device_export_handlers;
mod device_export_store;
pub(crate) mod device_registration_handlers;
mod device_registration_store;
mod error;
pub(crate) mod faulty_report_handlers;
mod faulty_report_store;
pub(crate) mod flashing_handlers;
mod flashing_store;
mod heartbeat_store;
mod legacy_relay_store;
pub(crate) mod log_handlers;
mod postgres;
mod registration_store;
mod relay_configuration_store;
pub(crate) mod relay_handlers;
mod relay_store;
mod repository;
pub(crate) mod repository_types;
mod routes;
pub(crate) mod test_catalog_handlers;
pub(crate) mod test_completion_handlers;
mod test_completion_store;
pub(crate) mod test_execution_handlers;
mod test_export;
pub(crate) mod test_export_handlers;
#[cfg(test)]
#[path = "tests/mod.rs"]
pub mod tests;
pub(crate) mod tus_handlers;
mod tus_store;
mod types;
pub(crate) mod uart_handlers;
mod upload_events;
mod validation;

mod artifact_inventory_handlers;
mod callbacks;
mod controller_handlers;
mod device_actions;
mod http_support;
mod inventory_handlers;
mod power_handlers;
mod relay_inventory_handlers;

pub(crate) use constants::{
    DEVICE_ACTION_HTTP_TIMEOUT, DEVICE_CALLBACK_TIMEOUT, DEVICE_COMMAND_RETRY_DELAY,
    EDGE_CONTROLLER_PORT, MAX_PAGE_SIZE, RELAY_CONTROLLER_TIMEOUT, TEST_CANCELLATION_TIMEOUT,
    UART_CONFIGURATION_TIMEOUT,
};
pub(crate) use error::DeviceRepositoryError;
#[cfg(test)]
use power_handlers::device_power_event;
pub(crate) use relay_handlers::relay_device_state;
pub(crate) use repository::DeviceRepository;
pub(crate) use repository_types::*;
pub(crate) use routes::router;
pub(crate) use types::{FlashConfirmationQuery, TtyEntry};

pub use types::{
    ActiveExecutionRecord, AdminAlertListQuery, AlertList, AllAlertListQuery, AvailableRelayDevice,
    AvailableRelayDevicesQuery, AvailableRelayDevicesResponse, BuildExecutionList, BuildFilters,
    BuildFlagRequest, BuildFlagResponse, BuildList, BuildListQuery, BuildUploadInitRequest,
    BuildUploadInitResponse, BuildUploadResponse, BuildsQuery, CallbackResponse, CallbackStatus,
    CompatibilityErrorResponse, ControllerEditRequest, ControllerHeartbeat, ControllerList,
    ControllerListQuery, ControllerRegistration, ControllerRegistrationResponse,
    CreateExecutionRequest, DefaultArtifactCopyQuery, DefaultArtifactCopyResponse, DeviceAction,
    DeviceActionRequest, DeviceController, DeviceCsvQuery, DeviceDataResponse,
    DeviceFlashingResponse, DeviceHeartbeatResponse, DeviceList, DeviceListQuery,
    DeviceRegistration, DeviceRegistrationResponse, DeviceStateAnalytics,
    DeviceStateDetailedAnalytics, DeviceStateDetailedItem, DeviceTopologyResponse,
    DeviceTypeFolder, DeviceTypeFolderRequest, EmptyObjectResponse, ExecutionCaseInput,
    ExecutionCaseRecord, ExecutionList, ExecutionListQuery, ExecutionReportRecord,
    ExecutionSelection, ExecutionSuiteSelection, FaultyReportCreateRequest, FaultyReportDetail,
    FaultyReportRecord, FaultyReportStatus, FaultyReportStatusRequest, FaultyReportUser,
    HeartbeatTimeoutRequest, LegacyRelayChannelListQuery, LegacyRelayConfiguration,
    LegacyRelayConfigurationResponse, LegacyRelayConflictQuery, LegacyRelayListQuery,
    LegacyRelayRemapRequest, LogCreateRequest, LogEntry, LogList, LogListQuery, MappingCallback,
    MessageResponse, PaginationResponse, Relay, RelayChannelConfiguration, RelayChannelRecord,
    RelayChannelRegistration, RelayConfigurationAccepted, RelayConfigurationConfirmation,
    RelayConfigurationRequest, RelayConfirmationResponse, RelayConflictResponse,
    RelayConflictState, RelayIdentityUpdateRequest, RelayIdentityUpdateResponse, RelayRecord,
    RelayRegistration, SuccessResponse, TestCompletionQuery, TestCompletionResponse,
    UartConfigurationRequest, UartConfigurationResponse, UpdateExecutionRequest,
    UserDeviceListQuery, VoltageLevel,
};

pub use artifact_inventory_handlers::*;
pub use callbacks::*;
pub use controller_handlers::*;
pub use device_actions::*;
pub(crate) use http_support::{error_response, repository_error};
pub use inventory_handlers::*;
pub(crate) use postgres::PostgresDeviceRepository;
pub use power_handlers::*;
pub use relay_inventory_handlers::*;
pub use uart_handlers::*;
