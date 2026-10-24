//! `ExecutionReportRepository` impl: report records and case payloads.
//!
//! Ports Node `TestService` / report-service handoff persistence.

use super::*;

use crate::persistence::entity::{execution_report, test_case, test_execution};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, LockType},
};

/// PURPOSE: Serialize `report json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
fn report_json(model: &execution_report::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "testExecutionId": model.test_execution_id,
        "status": match model.status {
            execution_report::ExecutionReportStatus::Generating => "generating",
            execution_report::ExecutionReportStatus::Uploading => "uploading",
            execution_report::ExecutionReportStatus::Failed => "failed",
            execution_report::ExecutionReportStatus::Completed => "completed",
            execution_report::ExecutionReportStatus::Uploaded => "uploaded",
        },
        "deviceType": model.device_type,
        "buildVersion": model.build_version,
        "createdBy": model.created_by,
        "uploadError": model.upload_error,
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
    })
}

/// PURPOSE: Serialize `test case json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
fn test_case_json(model: &test_case::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "testCaseId": model.test_case_id,
        "title": model.title,
        "executionId": model.execution_id,
        "result": model.result.as_ref().map(|r| match r {
            test_case::TestResult::Pass => "PASS",
            test_case::TestResult::Fail => "FAIL",
        }),
        "comment": model.comment,
        "cmd": model.cmd,
        "jiraDefect": model.jira_defect,
        "suiteId": model.suite_id,
        "scriptFile": model.script_file,
        "planId": model.plan_id,
        "priorityId": model.priority_id,
        "order": model.order,
        "suiteName": model.suite_name,
        "updatedAt": model.updated_at,
        "createdAt": model.created_at,
        "startedAt": model.started_at,
        "endedAt": model.ended_at,
        "outputFilePath": model.output_file_path,
        "dmesgFilePath": model.dmesg_file_path,
        "labels": model.labels,
        "preCondition": model.pre_condition,
    })
}

/// PURPOSE: Parse API/DB string into the `parse report status` domain enum/value.
/// USE: Store ingest paths converting wire strings to SeaORM enums.
fn parse_report_status(status: &str) -> Result<execution_report::ExecutionReportStatus, DeviceRepositoryError> {
    match status {
        "generating" => Ok(execution_report::ExecutionReportStatus::Generating),
        "uploading" => Ok(execution_report::ExecutionReportStatus::Uploading),
        "failed" => Ok(execution_report::ExecutionReportStatus::Failed),
        "completed" => Ok(execution_report::ExecutionReportStatus::Completed),
        "uploaded" => Ok(execution_report::ExecutionReportStatus::Uploaded),
        _ => Err(DeviceRepositoryError::Validation(format!(
            "invalid execution report status: {status}"
        ))),
    }
}

#[async_trait]
impl ExecutionReportRepository for PostgresDeviceRepository {
    /// PURPOSE: Resolve edge/controller targets for `report generation target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn report_generation_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_reports",
            "report_generation_target",
        );
        Ok(test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(test_id))
            .filter(test_execution::Column::Status.eq(test_execution::TestStatus::Completed))
            .select_only()
            .column(test_execution::Column::DeviceType)
            .column(test_execution::Column::BuildVersion)
            .into_tuple::<(String, Option<String>)>()
            .one(self.orm())
            .await?
            .map(|(device_type, build_version)| {
                (device_type, build_version.unwrap_or_default())
            }))
    }

    /// PURPOSE: Create/persist `create execution report record`.
    /// USE: Create handlers after validation.
    async fn create_execution_report_record(
        &self,
        report_id: Uuid,
        test_id: &str,
        device_type: &str,
        build_version: &str,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_reports",
            "create_execution_report_record",
        );
        let transaction = self.orm().begin().await?;
        let active = execution_report::Entity::find()
            .filter(execution_report::Column::TestExecutionId.eq(test_id))
            .filter(
                execution_report::Column::Status.is_in([
                    execution_report::ExecutionReportStatus::Generating,
                    execution_report::ExecutionReportStatus::Uploading,
                ]),
            )
            // Row lock: serialize concurrent updates on this controller/device/relay.
            .lock(LockType::Update)
            .one(&transaction)
            .await?;
        if let Some(active) = active {
            let status = match active.status {
                execution_report::ExecutionReportStatus::Generating => "generating",
                execution_report::ExecutionReportStatus::Uploading => "uploading",
                other => {
                    return Err(DeviceRepositoryError::Conflict(format!(
                        "A report for this execution is already {other:?}"
                    )));
                }
            };
            return Err(DeviceRepositoryError::Conflict(format!(
                "A report for this execution is already {status}"
            )));
        }
        execution_report::Entity::delete_many()
            .filter(execution_report::Column::TestExecutionId.eq(test_id))
            .exec(&transaction)
            .await?;
        let now = chrono::Utc::now();
        let report = execution_report::ActiveModel {
            id: Set(report_id),
            test_execution_id: Set(test_id.to_owned()),
            status: Set(execution_report::ExecutionReportStatus::Generating),
            device_type: Set(device_type.to_owned()),
            build_version: Set(build_version.to_owned()),
            created_by: Set(user_id.to_owned()),
            upload_error: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        transaction.commit().await?;
        Ok(report_json(&report))
    }

    /// PURPOSE: Mutate persisted state for `update execution report status`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn update_execution_report_status(
        &self,
        report_id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_reports",
            "update_execution_report_status",
        );
        let status = parse_report_status(status)?;
        execution_report::Entity::update_many()
            .col_expr(execution_report::Column::Status, crate::persistence::pg_enum::pg_enum(&status))
            .col_expr(
                execution_report::Column::UploadError,
                Expr::value(error.map(str::to_owned)),
            )
            .col_expr(
                execution_report::Column::UpdatedAt,
                Expr::value(chrono::Utc::now()),
            )
            .filter(execution_report::Column::Id.eq(report_id))
            .exec(self.orm())
            .await?;
        Ok(())
    }

    /// PURPOSE: Implement `execution_report` domain/persistence logic.
    /// USE: Called as `execution_report` from handlers, repository impls, or sibling helpers.
    async fn execution_report(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_reports",
            "execution_report",
        );
        Ok(execution_report::Entity::find()
            .filter(execution_report::Column::TestExecutionId.eq(test_id))
            .order_by_desc(execution_report::Column::CreatedAt)
            .one(self.orm())
            .await?
            .map(|model| report_json(&model)))
    }

    /// PURPOSE: Resolve edge/controller targets for `execution report target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn execution_report_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_reports",
            "execution_report_target",
        );
        Ok(test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(test_id))
            .select_only()
            .column(test_execution::Column::DeviceType)
            .column(test_execution::Column::BuildVersion)
            .into_tuple::<(String, Option<String>)>()
            .one(self.orm())
            .await?
            .map(|(device_type, build_version)| {
                (device_type, build_version.unwrap_or_default())
            }))
    }

    /// PURPOSE: Implement `test_case_log_path` domain/persistence logic.
    /// USE: Called as `test_case_log_path` from handlers, repository impls, or sibling helpers.
    async fn test_case_log_path(
        &self,
        case_id: i64,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_reports",
            "test_case_log_path",
        );
        let id = i32::try_from(case_id).map_err(|_| {
            DeviceRepositoryError::Validation(format!("invalid test case id: {case_id}"))
        })?;
        Ok(test_case::Entity::find_by_id(id)
            .one(self.orm())
            .await?
            .and_then(|model| model.output_file_path))
    }

    /// PURPOSE: Implement `execution_cases` domain/persistence logic.
    /// USE: Called as `execution_cases` from handlers, repository impls, or sibling helpers.
    async fn execution_cases(
        &self,
        user_id: &str,
        test_id: &str,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_reports",
            "execution_cases",
        );
        let owned = test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(test_id))
            .filter(test_execution::Column::CreatedBy.eq(user_id))
            .count(self.orm())
            .await?
            > 0;
        if !owned {
            return Ok(None);
        }
        Ok(Some(
            test_case::Entity::find()
                .filter(test_case::Column::ExecutionId.eq(test_id))
                .order_by_asc(test_case::Column::Id)
                .all(self.orm())
                .await?
                .into_iter()
                .map(|model| {
                    serde_json::json!({
                        "testCaseId": model.test_case_id,
                        "suiteId": model.suite_id,
                        "result": model.result.as_ref().map(|r| match r {
                            test_case::TestResult::Pass => "PASS",
                            test_case::TestResult::Fail => "FAIL",
                        }),
                        "comment": model.comment,
                        "jiraDefect": model.jira_defect,
                        "title": model.title,
                        "suiteName": model.suite_name,
                        "scriptFile": model.script_file,
                        "id": model.id,
                        "updatedAt": model.updated_at,
                        "outputFilePath": model.output_file_path,
                        "dmesgFilePath": model.dmesg_file_path,
                    })
                })
                .collect(),
        ))
    }

    /// PURPOSE: Implement `test_case` domain/persistence logic.
    /// USE: Called as `test_case` from handlers, repository impls, or sibling helpers.
    async fn test_case(
        &self,
        case_id: i64,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_reports",
            "test_case",
        );
        let id = i32::try_from(case_id).map_err(|_| {
            DeviceRepositoryError::Validation(format!("invalid test case id: {case_id}"))
        })?;
        Ok(test_case::Entity::find_by_id(id)
            .one(self.orm())
            .await?
            .map(|model| test_case_json(&model)))
    }
}
