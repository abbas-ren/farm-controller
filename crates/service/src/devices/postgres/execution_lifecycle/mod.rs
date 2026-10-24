//! `ExecutionLifecycleRepository` impl: create, cancel, logs, status updates.
//!
//! Child modules own creation, cancellation, and log persistence. Ports Node
//! `TestService` execution lifecycle.

use super::*;

use crate::persistence::entity::test_execution;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};

mod cancellation;
mod creation;
mod logs;

#[async_trait]
impl ExecutionLifecycleRepository for PostgresDeviceRepository {
    /// PURPOSE: Create/persist `create test execution`.
    /// USE: Create handlers after validation.
    async fn create_test_execution(
        &self,
        execution: &ExecutionCreation,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::mod", "create_test_execution");
        self.create_test_execution_inner(execution).await
    }

    /// PURPOSE: Implement `test_execution_build_version` domain/persistence logic.
    /// USE: Called as `test_execution_build_version` from handlers, repository impls, or sibling helpers.
    async fn test_execution_build_version(
        &self,
        test_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::mod", "test_execution_build_version");
        self.test_execution_build_version_inner(test_id).await
    }

    /// PURPOSE: Create/persist `create log`.
    /// USE: Create handlers after validation.
    async fn create_log(
        &self,
        request: &LogCreateRequest,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::mod", "create_log");
        self.create_log_inner(request).await
    }

    /// PURPOSE: List/filter `list logs` rows for the API.
    /// USE: HTTP list handlers and repository trait methods.
    async fn list_logs(
        &self,
        query: &LogListQuery,
        search: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::mod", "list_logs");
        self.list_logs_inner(query, search).await
    }

    /// PURPOSE: Implement `executions_by_build` domain/persistence logic.
    /// USE: Called as `executions_by_build` from handlers, repository impls, or sibling helpers.
    async fn executions_by_build(
        &self,
        build_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::mod", "executions_by_build");
        self.executions_by_build_inner(build_id, limit, offset)
            .await
    }

    /// PURPOSE: Implement `test_results` domain/persistence logic.
    /// USE: Called as `test_results` from handlers, repository impls, or sibling helpers.
    async fn test_results(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::mod", "test_results");
        self.test_results_inner(test_id).await
    }

    /// PURPOSE: Mutate persisted state for `update execution status`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn update_execution_status(
        &self,
        test_id: &str,
        status: &str,
    ) -> Result<bool, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::mod", "update_execution_status");
        let status_enum = parse_test_status(status)?;
        let now = chrono::Utc::now();
        let mut update = test_execution::Entity::update_many()
            .col_expr(
                test_execution::Column::Status,
                Expr::value(Some(status_enum)),
            )
            .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
            .filter(test_execution::Column::TestId.eq(test_id));

        if status == "in_progress" {
            update = update.col_expr(
                test_execution::Column::StartedAt,
                Expr::cust("COALESCE(\"startedAt\", now())"),
            );
        }
        if matches!(status, "completed" | "cancelled" | "failed") {
            update = update.col_expr(
                test_execution::Column::EndedAt,
                Expr::value(Some(now)),
            );
        } else {
            update = update.col_expr(
                test_execution::Column::EndedAt,
                Expr::value(Option::<chrono::DateTime<chrono::Utc>>::None),
            );
        }

        let result = update.exec(self.orm()).await?;
        Ok(result.rows_affected > 0)
    }

    /// PURPOSE: Implement `request_test_cancellation` domain/persistence logic.
    /// USE: Called as `request_test_cancellation` from handlers, repository impls, or sibling helpers.
    async fn request_test_cancellation(
        &self,
        test_id: &str,
        user_id: &str,
        nfs_host_path: &str,
    ) -> Result<Option<TestCancellationResult>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::mod", "request_test_cancellation");
        self.request_test_cancellation_inner(test_id, user_id, nfs_host_path)
            .await
    }
}

/// PURPOSE: Parse API/DB string into the `parse test status` domain enum/value.
/// USE: Store ingest paths converting wire strings to SeaORM enums.
fn parse_test_status(status: &str) -> Result<test_execution::TestStatus, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::mod", "parse_test_status");
    match status {
        "not_executed" => Ok(test_execution::TestStatus::NotExecuted),
        "in_progress" => Ok(test_execution::TestStatus::InProgress),
        "completed" => Ok(test_execution::TestStatus::Completed),
        "queued" => Ok(test_execution::TestStatus::Queued),
        "cancelled" => Ok(test_execution::TestStatus::Cancelled),
        "failed" => Ok(test_execution::TestStatus::Failed),
        _ => Err(DeviceRepositoryError::Validation(format!(
            "invalid test status: {status}"
        ))),
    }
}
