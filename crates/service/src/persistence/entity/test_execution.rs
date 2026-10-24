//! Test execution aggregates (`test_executions` table).
//!
//! Ports `TestExecution.ts` / `TestService` and TestWorker phase machinery.
//! Each row is a farm run: selected plan/suites, assigned device, build, and a
//! fine-grained [`ExecutionPhase`] that workers advance through flash/test/cancel.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Case-selection payload persisted with the execution (UI → worker contract).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionInput {
    pub mode: String,
    pub plan_id: String,
    pub plan_name: Option<String>,
    pub exclude: Option<SelectionExclude>,
    pub suites: Option<Vec<SelectionSuite>>,
}

/// Suites/cases excluded from an inclusive selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionExclude {
    pub suites: Option<Vec<String>>,
    pub cases_by_suite: Option<std::collections::HashMap<String, Vec<String>>>,
}

/// One suite contribution inside [`SelectionInput`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionSuite {
    pub suite_id: String,
    pub select_all: bool,
    pub cases: Option<Vec<String>>,
}

/// High-level execution status shown in the UI and device mirrors.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_test_executions_status"
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

/// Worker state-machine phase for preparing, flashing, and running tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_test_executions_executionPhase"
)]
pub enum ExecutionPhase {
    #[sea_orm(string_value = "PREPARE_ARTIFACTS")]
    PrepareArtifacts,
    #[sea_orm(string_value = "PREPARE_TEST_SCRIPTS")]
    PrepareTestScripts,
    #[sea_orm(string_value = "QUEUE_TEST_EXECUTION")]
    QueueTestExecution,
    #[sea_orm(string_value = "PROCESSING_QUEUED")]
    ProcessingQueued,
    #[sea_orm(string_value = "ASSIGNING_DEVICE")]
    AssigningDevice,
    #[sea_orm(string_value = "DEVICE_ASSIGNED")]
    DeviceAssigned,
    #[sea_orm(string_value = "SEND_IPL_REQUEST")]
    SendIplRequest,
    #[sea_orm(string_value = "WAIT_FOR_IPL_CONFIRMATION")]
    WaitForIplConfirmation,
    #[sea_orm(string_value = "REQUEUE_AFTER_IPL_CONFIRMATION")]
    RequeueAfterIplConfirmation,
    #[sea_orm(string_value = "SEND_FLASH_REQUEST")]
    SendFlashRequest,
    #[sea_orm(string_value = "WAIT_FOR_DEVICE_BOOT")]
    WaitForDeviceBoot,
    #[sea_orm(string_value = "START_TEST_EXECUTION")]
    StartTestExecution,
    #[sea_orm(string_value = "SEND_FALLBACK_FLASH_REQUEST")]
    SendFallbackFlashRequest,
    #[sea_orm(string_value = "WAIT_FOR_FALLBACK_COMPLETION")]
    WaitForFallbackCompletion,
    #[sea_orm(string_value = "COMPLETE_SUCCESS")]
    CompleteSuccess,
    #[sea_orm(string_value = "COMPLETE_FAILED")]
    CompleteFailed,
    #[sea_orm(string_value = "COMPLETE_CANCELLED")]
    CompleteCancelled,
}

/// One scheduled or in-flight test run on the farm.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "test_executions", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    /// Stable business key used across queues, cases, and reports.
    #[sea_orm(unique)]
    pub test_id: String,
    pub device_family: String,
    pub device_type: String,
    pub device_id: Option<String>,
    pub build_id: String,
    pub build_version: Option<String>,
    pub test_plan_id: i64,
    /// Selected suite metadata JSON for UI/history.
    pub test_suits: Json,
    /// Optional typed selection payload (also see [`SelectionInput`]).
    pub selection_input: Option<Json>,
    pub logs: Option<Json>,
    pub test_cycle_id: Option<String>,
    pub status: Option<TestStatus>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub started_at: Option<DateTimeUtc>,
    pub ended_at: Option<DateTimeUtc>,
    pub test_plan_name: Option<String>,
    pub name: Option<String>,
    /// Linked `device_action_queue` id when queued.
    pub queue_id: Option<String>,
    pub is_all_selected: Option<bool>,
    pub execution_phase: Option<ExecutionPhase>,
    pub cancel_requested: Option<bool>,
    pub cancel_requested_by: Option<String>,
    pub cancel_requested_at: Option<DateTimeUtc>,
    pub cancel_handled: Option<bool>,
    /// When true, cancel keeps artifacts instead of scrubbing.
    pub cancel_preserve: Option<bool>,
    pub cancel_system_reason: Option<String>,
    pub rtos_log_path: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::device::Entity",
        from = "Column::DeviceId",
        to = "super::device::Column::DeviceId"
    )]
    Device,
    #[sea_orm(
        has_many = "super::device_action_queue::Entity",
        from = "Column::TestId",
        to = "super::device_action_queue::Column::TestId"
    )]
    DeviceActionQueues,
    #[sea_orm(
        has_many = "super::test_case::Entity",
        from = "Column::TestId",
        to = "super::test_case::Column::ExecutionId"
    )]
    TestCases,
    #[sea_orm(
        has_one = "super::execution_report::Entity",
        from = "Column::TestId",
        to = "super::execution_report::Column::TestExecutionId"
    )]
    ExecutionReport,
    #[sea_orm(
        has_many = "super::faulty_report::Entity",
        from = "Column::TestId",
        to = "super::faulty_report::Column::LastTestExecutionId"
    )]
    FaultyReports,
}

impl Related<super::device::Entity> for Entity {
    /// PURPOSE: Join path to the assigned DUT.
    /// USE: Load device state when advancing execution phases.
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}

impl Related<super::device_action_queue::Entity> for Entity {
    /// PURPOSE: Join path to queue jobs for this execution.
    /// USE: Correlate dispatcher jobs with the parent run.
    fn to() -> RelationDef {
        Relation::DeviceActionQueues.def()
    }
}

impl Related<super::test_case::Entity> for Entity {
    /// PURPOSE: Join path to expanded case rows.
    /// USE: Aggregate results or update per-case outcomes.
    fn to() -> RelationDef {
        Relation::TestCases.def()
    }
}

impl Related<super::execution_report::Entity> for Entity {
    /// PURPOSE: Join path to the execution's report workflow row.
    /// USE: Check report status after completion.
    fn to() -> RelationDef {
        Relation::ExecutionReport.def()
    }
}

impl Related<super::faulty_report::Entity> for Entity {
    /// PURPOSE: Join path to faulty reports that reference this execution.
    /// USE: Surface related defect filings from an execution detail view.
    fn to() -> RelationDef {
        Relation::FaultyReports.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
