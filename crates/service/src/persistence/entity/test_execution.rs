use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionInput {
    pub mode: String,
    pub plan_id: String,
    pub plan_name: Option<String>,
    pub exclude: Option<SelectionExclude>,
    pub suites: Option<Vec<SelectionSuite>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionExclude {
    pub suites: Option<Vec<String>>,
    pub cases_by_suite: Option<std::collections::HashMap<String, Vec<String>>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionSuite {
    pub suite_id: String,
    pub select_all: bool,
    pub cases: Option<Vec<String>>,
}

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

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "test_executions", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub test_id: String,
    pub device_family: String,
    pub device_type: String,
    pub device_id: Option<String>,
    pub build_id: String,
    pub build_version: Option<String>,
    pub test_plan_id: i64,
    pub test_suits: Json,
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
    pub queue_id: Option<String>,
    pub is_all_selected: Option<bool>,
    pub execution_phase: Option<ExecutionPhase>,
    pub cancel_requested: Option<bool>,
    pub cancel_requested_by: Option<String>,
    pub cancel_requested_at: Option<DateTimeUtc>,
    pub cancel_handled: Option<bool>,
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
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}
impl Related<super::device_action_queue::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::DeviceActionQueues.def()
    }
}
impl Related<super::test_case::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::TestCases.def()
    }
}
impl Related<super::execution_report::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ExecutionReport.def()
    }
}
impl Related<super::faulty_report::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::FaultyReports.def()
    }
}
impl ActiveModelBehavior for ActiveModel {}
