//! Faulty-build reports (`faulty_reports` table).
//!
//! Ports `FaultyReport.ts` / `FaultyReportService`. Operators file these when a
//! release misbehaves on a device type; approval workflow can mark the release
//! faulty and attach evidence paths.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Review workflow status for a faulty report.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_faulty_reports_status"
)]
pub enum FaultyStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "approved")]
    Approved,
    #[sea_orm(string_value = "rejected")]
    Rejected,
}

/// One operator-submitted faulty report against a release.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "faulty_reports", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub release_id: Uuid,
    pub device_type: String,
    pub device_family: String,
    pub last_test_execution_id: Option<String>,
    pub description: String,
    pub file_path: Option<String>,
    pub logs_path: Option<String>,
    pub status: FaultyStatus,
    pub created_by: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::release::Entity",
        from = "Column::ReleaseId",
        to = "super::release::Column::Id"
    )]
    Release,
    #[sea_orm(
        belongs_to = "super::test_execution::Entity",
        from = "Column::LastTestExecutionId",
        to = "super::test_execution::Column::TestId"
    )]
    TestExecution,
}

impl Related<super::release::Entity> for Entity {
    /// PURPOSE: Join path to the implicated release.
    /// USE: Load release version/tag when reviewing or listing faulty reports.
    fn to() -> RelationDef {
        Relation::Release.def()
    }
}

impl Related<super::test_execution::Entity> for Entity {
    /// PURPOSE: Join path to the last related test execution.
    /// USE: Attach execution context and logs when investigating a report.
    fn to() -> RelationDef {
        Relation::TestExecution.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
