//! Report test-case and terminal-status persistence boundary.

use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, sea_query::Expr,
};
use uuid::Uuid;

use crate::persistence::entity::{execution_report, test_case};

use super::{ReportError, ReportStatus, TestCaseRecord};

#[async_trait]
pub trait ReportDataSource: Send + Sync {
    /// Load test-case rows for the execution being reported.
    async fn test_cases(&self, test_id: &str) -> Result<Vec<TestCaseRecord>, ReportError>;

    /// Update report status.
    async fn update_report_status(
        &self,
        _report_id: Uuid,
        _status: ReportStatus,
        _error: Option<&str>,
    ) -> Result<(), ReportError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("reports::data_source", "update_report_status");
        Ok(())
    }
}

pub struct PostgresReportDataSource {
    orm: DatabaseConnection,
}

impl PostgresReportDataSource {
    /// Wrap a SeaORM connection for report queries.
    pub fn new(orm: DatabaseConnection) -> Self {
        Self { orm }
    }
}

#[async_trait]
impl ReportDataSource for PostgresReportDataSource {
    /// Load test-case rows for the execution being reported.
    async fn test_cases(&self, test_id: &str) -> Result<Vec<TestCaseRecord>, ReportError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("reports::data_source", "test_cases");
        let rows = test_case::Entity::find()
            .filter(test_case::Column::ExecutionId.eq(test_id))
            .order_by_asc(test_case::Column::SuiteId)
            .order_by_asc(test_case::Column::Id)
            .all(&self.orm)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| TestCaseRecord {
                test_case_id: row.test_case_id,
                suite_name: row.suite_name,
                result: row.result.map(|r| match r {
                    test_case::TestResult::Pass => "PASS".to_owned(),
                    test_case::TestResult::Fail => "FAIL".to_owned(),
                }),
                comment: row.comment,
                command: row.cmd,
                jira_defect: row.jira_defect,
                output_file_path: row.output_file_path,
                labels: row.labels,
            })
            .collect())
    }

    /// Update report status.
    async fn update_report_status(
        &self,
        report_id: Uuid,
        status: ReportStatus,
        error: Option<&str>,
    ) -> Result<(), ReportError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("reports::data_source", "update_report_status");
        let status = match status {
            ReportStatus::Completed => execution_report::ExecutionReportStatus::Completed,
            ReportStatus::Failed => execution_report::ExecutionReportStatus::Failed,
            _ => return Ok(()),
        };
        let now = Utc::now();
        execution_report::Entity::update_many()
            .col_expr(execution_report::Column::Status, crate::persistence::pg_enum::pg_enum(&status))
            .col_expr(
                execution_report::Column::UploadError,
                Expr::value(error.map(str::to_owned)),
            )
            .col_expr(execution_report::Column::UpdatedAt, Expr::value(now))
            .filter(execution_report::Column::Id.eq(report_id))
            .exec(&self.orm)
            .await?;
        Ok(())
    }
}
