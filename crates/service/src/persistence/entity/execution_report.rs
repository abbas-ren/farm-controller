//! Test execution report upload tracking (`execution_reports` table).
//!
//! Ports `ExecutionReport.ts` / report worker flows. Tracks generation and
//! upload of aggregated reports for a completed (or completing) test execution.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Pipeline stage for building and publishing an execution report.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_execution_reports_status"
)]
pub enum ExecutionReportStatus {
    #[sea_orm(string_value = "generating")]
    Generating,
    #[sea_orm(string_value = "uploading")]
    Uploading,
    #[sea_orm(string_value = "failed")]
    Failed,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "uploaded")]
    Uploaded,
}

/// One report artifact workflow bound to a test execution.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "execution_reports", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Matches `test_executions.testId`.
    pub test_execution_id: String,
    pub status: ExecutionReportStatus,
    pub device_type: String,
    pub build_version: String,
    pub created_by: String,
    pub upload_error: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::test_execution::Entity",
        from = "Column::TestExecutionId",
        to = "super::test_execution::Column::TestId"
    )]
    TestExecution,
}

impl Related<super::test_execution::Entity> for Entity {
    /// PURPOSE: Join path to the execution being reported.
    /// USE: Load execution metadata when generating or displaying reports.
    fn to() -> RelationDef {
        Relation::TestExecution.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
