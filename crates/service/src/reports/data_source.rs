//! Report test-case and terminal-status persistence boundary.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{ReportError, ReportStatus, TestCaseRecord};

#[async_trait]
pub trait ReportDataSource: Send + Sync {
    async fn test_cases(&self, test_id: &str) -> Result<Vec<TestCaseRecord>, ReportError>;

    async fn update_report_status(
        &self,
        _report_id: Uuid,
        _status: ReportStatus,
        _error: Option<&str>,
    ) -> Result<(), ReportError> {
        Ok(())
    }
}

pub struct PostgresReportDataSource {
    pool: PgPool,
}

impl PostgresReportDataSource {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReportDataSource for PostgresReportDataSource {
    async fn test_cases(&self, test_id: &str) -> Result<Vec<TestCaseRecord>, ReportError> {
        Ok(sqlx::query_as::<_, TestCaseRecord>(
            r#"
            SELECT "testCaseId" AS test_case_id, "suiteName" AS suite_name,
                   result::text, comment, cmd AS command, "jiraDefect" AS jira_defect,
                   "outputFilePath" AS output_file_path, labels
            FROM testcase
            WHERE "executionId" = $1
            ORDER BY "suiteId", id
            "#,
        )
        .bind(test_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn update_report_status(
        &self,
        report_id: Uuid,
        status: ReportStatus,
        error: Option<&str>,
    ) -> Result<(), ReportError> {
        let status = match status {
            ReportStatus::Completed => "completed",
            ReportStatus::Failed => "failed",
            _ => return Ok(()),
        };
        sqlx::query(
            r#"UPDATE execution_reports
               SET status = $2::"enum_execution_reports_status", "uploadError" = $3,
                   "updatedAt" = now()
               WHERE id = $1"#,
        )
        .bind(report_id)
        .bind(status)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
