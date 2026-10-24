use super::*;

use crate::persistence::entity::{release, test_case, test_execution};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseBackend, EntityTrait, FromQueryResult, PaginatorTrait,
    QueryFilter, QuerySelect, Set, Statement, TransactionTrait,
    sea_query::LockType,
};
use uuid::Uuid;

impl PostgresDeviceRepository {
    pub(super) async fn create_test_execution_inner(
        &self,
        execution: &ExecutionCreation,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::creation", "create_test_execution_inner");
        let transaction = self.orm().begin().await?;
        let release_id = Uuid::parse_str(&execution.build_id).map_err(|_| {
            DeviceRepositoryError::NotFound(format!("Release {} not found", execution.build_id))
        })?;
        let release = release::Entity::find_by_id(release_id)
            .lock(LockType::Share)
            .one(&transaction)
            .await?
            .ok_or_else(|| {
                DeviceRepositoryError::NotFound(format!("Release {} not found", execution.build_id))
            })?;
        if release.is_faulty == Some(true) {
            return Err(DeviceRepositoryError::Validation(format!(
                "Cannot create test execution: Release {} is marked as faulty",
                execution.build_id
            )));
        }
        let test_id = Uuid::new_v4().simple().to_string();
        let name = execution
            .name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "{} | {}:{} | {}",
                    execution.plan_name,
                    execution.device_type,
                    release.version,
                    chrono::Utc::now().format("%b %-d, %Y %-I:%M %p")
                )
            });
        let test_suites = serde_json::to_value(&execution.test_suites)
            .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
        let selection = execution
            .selection
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
        let now = chrono::Utc::now();
        test_execution::ActiveModel {
            test_id: Set(test_id.clone()),
            device_family: Set(execution.device_family.clone()),
            device_type: Set(execution.device_type.clone()),
            device_id: Set(None),
            build_id: Set(execution.build_id.clone()),
            build_version: Set(Some(release.version.clone())),
            test_plan_id: Set(execution.plan_id),
            test_suits: Set(test_suites),
            selection_input: Set(selection),
            name: Set(Some(name)),
            status: Set(Some(test_execution::TestStatus::NotExecuted)),
            created_by: Set(Some(execution.user_id.clone())),
            test_plan_name: Set(Some(execution.plan_name.clone())),
            is_all_selected: Set(Some(execution.is_all_selected)),
            execution_phase: Set(Some(test_execution::ExecutionPhase::PrepareArtifacts)),
            logs: Set(Some(serde_json::json!([]))),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&transaction)
        .await?;

        for case in &execution.cases {
            let result = match case.result.as_deref() {
                Some("PASS") => Some(test_case::TestResult::Pass),
                Some("FAIL") => Some(test_case::TestResult::Fail),
                _ => None,
            };
            test_case::ActiveModel {
                test_case_id: Set(case.test_case_id),
                title: Set(case.title.clone()),
                execution_id: Set(Some(test_id.clone())),
                result: Set(result),
                suite_id: Set(case.suite_id),
                script_file: Set(Some(case.script_file.clone())),
                plan_id: Set(case.plan_id),
                priority_id: Set(case.priority_id.and_then(|v| i32::try_from(v).ok())),
                order: Set(case.order.and_then(|v| i32::try_from(v).ok())),
                suite_name: Set(case.suite_name.clone()),
                labels: Set(Some(case.labels.clone())),
                pre_condition: Set(case.pre_condition.clone()),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(&transaction)
            .await?;
        }

        // Nested to_jsonb + jsonb_agg payload shape kept as raw SQL for API compatibility.
        #[derive(Debug, FromQueryResult)]
        struct CreatedExecution {
            value: serde_json::Value,
        }
        let created = CreatedExecution::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT to_jsonb(te) || jsonb_build_object(
                 'Device', NULL,
                 'testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc."suiteId", tc.id)
                                        FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb),
                 'logs', '[]'::jsonb) AS value
               FROM test_executions te WHERE te."testId" = $1"#,
            [test_id.into()],
        ))
        .one(&transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::Internal("created test execution disappeared".to_owned())
        })?;

        transaction.commit().await?;
        Ok(created.value)
    }

    pub(super) async fn test_execution_build_version_inner(
        &self,
        test_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::creation", "test_execution_build_version_inner");
        Ok(test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(test_id))
            .select_only()
            .column(test_execution::Column::BuildVersion)
            .into_tuple::<Option<String>>()
            .one(self.orm())
            .await?
            .flatten())
    }

    pub(super) async fn executions_by_build_inner(
        &self,
        build_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::creation", "executions_by_build_inner");
        let total = test_execution::Entity::find()
            .filter(test_execution::Column::BuildId.eq(build_id))
            .count(self.orm())
            .await? as i64;

        // Nested to_jsonb + jsonb_agg kept as raw SQL for API response shape.
        #[derive(Debug, FromQueryResult)]
        struct ExecutionRow {
            value: serde_json::Value,
        }
        let executions = ExecutionRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT to_jsonb(te) || jsonb_build_object('testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id) FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb)) AS value
               FROM test_executions te WHERE te."buildId" = $1 ORDER BY te."createdAt" DESC LIMIT $2 OFFSET $3"#,
            [build_id.into(), limit.into(), offset.into()],
        ))
        .all(self.orm())
        .await?
        .into_iter()
        .map(|row| row.value)
        .collect::<Vec<_>>();

        Ok(serde_json::json!({
            "limit": limit,
            "offset": offset,
            "total": total,
            "executions": executions
        }))
    }

    pub(super) async fn test_results_inner(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::creation", "test_results_inner");
        // Nested jsonb_agg kept as raw SQL for API response shape.
        #[derive(Debug, FromQueryResult)]
        struct ResultsRow {
            value: serde_json::Value,
        }
        Ok(
            ResultsRow::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT jsonb_build_object('status', te.status, 'testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id) FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb)) AS value
                   FROM test_executions te WHERE te."testId" = $1"#,
                [test_id.into()],
            ))
            .one(self.orm())
            .await?
            .map(|row| row.value),
        )
    }
}
