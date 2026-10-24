//! Device inventory rows (`devices` table).
//!
//! Ports `farm_controller_old/device-service/src/models/DeviceModel.ts`. Each row
//! is a DUT registered under a device controller, with runtime state used by
//! assignment, flash, and test workers.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Live availability of a device for scheduling.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "enum_devices_state")]
pub enum DeviceState {
    /// Currently running a job.
    #[sea_orm(string_value = "busy")]
    Busy,
    /// Heartbeat / connectivity lost.
    #[sea_orm(string_value = "not_reachable")]
    NotReachable,
    /// Idle and assignable.
    #[sea_orm(string_value = "free")]
    Free,
    /// Marked faulty pending review.
    #[sea_orm(string_value = "faulty")]
    Faulty,
    /// State not yet determined.
    #[sea_orm(string_value = "unknown")]
    Unknown,
}

/// Administrative approval workflow for onboarding a device.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_devices_status"
)]
pub enum DeviceStatus {
    /// Allowed to participate in farm operations.
    #[sea_orm(string_value = "approved")]
    Approved,
    /// Awaiting operator approval.
    #[sea_orm(string_value = "requested")]
    Requested,
    /// Rejected from the farm.
    #[sea_orm(string_value = "declined")]
    Declined,
}

/// Last known test-execution outcome mirrored onto the device row.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_devices_lastExecutionStatus"
)]
pub enum TestStatus {
    #[sea_orm(string_value = "not_executed")]
    NotExecuted,
    #[sea_orm(string_value = "in_progress")]
    InProgress,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "queued")]
    Queued,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
    #[sea_orm(string_value = "failed")]
    Failed,
}

/// Power-relay view of whether the DUT is powered on.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "enum_devices_power")]
pub enum DevicePower {
    #[sea_orm(string_value = "on")]
    On,
    #[sea_orm(string_value = "off")]
    Off,
}

/// One physical or virtual DUT in the farm inventory.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "devices", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub device_name: Option<String>,
    pub device_family: Option<String>,
    /// Stable business key used across queues and executions.
    #[sea_orm(unique)]
    pub device_id: String,
    pub mac_address: Option<String>,
    pub ip_address: Option<String>,
    pub device_type: Option<String>,
    pub state: Option<DeviceState>,
    pub status: Option<DeviceStatus>,
    pub last_connected_on: DateTimeUtc,
    /// Owning device-controller identifier.
    pub controller_id: Option<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub status_action_by: Option<String>,
    pub last_test_execution: Option<String>,
    pub last_execution_status: Option<TestStatus>,
    pub state_updated_at: Option<DateTimeUtc>,
    pub build_id: Option<String>,
    /// Expected heartbeat interval in seconds for liveness checks.
    pub heartbeat_timer: i32,
    pub upgrading: Option<bool>,
    pub flashing: Option<bool>,
    pub software_version: Option<String>,
    pub power: Option<DevicePower>,
    /// NFS mount path used when staging artifacts onto the device.
    pub nfs_path: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub deleted_at: Option<DateTimeUtc>,
}

/// Relations used by SeaORM joins from devices to child tables.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        has_many = "super::device_interface::Entity",
        from = "Column::DeviceId",
        to = "super::device_interface::Column::DeviceId"
    )]
    Interfaces,
    #[sea_orm(
        has_many = "super::test_execution::Entity",
        from = "Column::DeviceId",
        to = "super::test_execution::Column::DeviceId"
    )]
    TestExecutions,
    #[sea_orm(
        has_many = "super::device_action_queue::Entity",
        from = "Column::DeviceId",
        to = "super::device_action_queue::Column::TargetDeviceId"
    )]
    DeviceActionQueues,
}

impl Related<super::device_interface::Entity> for Entity {
    /// PURPOSE: Define the SeaORM join path to device interfaces.
    /// USE: Invoked by `.find_also_related` / loader helpers when expanding interfaces.
    fn to() -> RelationDef {
        Relation::Interfaces.def()
    }
}

impl Related<super::test_execution::Entity> for Entity {
    /// PURPOSE: Define the SeaORM join path to test executions assigned to this device.
    /// USE: Invoked when loading execution history for a device.
    fn to() -> RelationDef {
        Relation::TestExecutions.def()
    }
}

impl Related<super::device_action_queue::Entity> for Entity {
    /// PURPOSE: Define the SeaORM join path to queued actions targeting this device.
    /// USE: Invoked when correlating queue jobs with their target DUT.
    fn to() -> RelationDef {
        Relation::DeviceActionQueues.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
