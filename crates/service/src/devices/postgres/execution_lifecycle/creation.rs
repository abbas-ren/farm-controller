//! Create test executions and linked test-case rows.
//!
//! Ports Node `TestService.createTestExecution`.

use super::*;

use crate::{
    devices::postgres::json_models::{
        created_execution_json, execution_with_cases_json, test_case_json, test_status_str,
    },
    persistence::entity::{release, test_case, test_execution},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
    sea_query::LockType,
};
use uuid::Uuid;

impl PostgresDeviceRepository {
    /// PURPOSE: Create/persist `create test execution inner`.
    /// USE: Create handlers after validation.
    pub(super) async fn create_test_execution_inner(
        &self,
        execution: &ExecutionCreation,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_lifecycle::creation",
            "create_test_execution_inner",
        );
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
        if release.is_faulty {
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

        let created_model = test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(&test_id))
            .one(&transaction)
            .await?
            .ok_or_else(|| {
                DeviceRepositoryError::Internal("created test execution disappeared".to_owned())
            })?;
        let mut cases = test_case::Entity::find()
            .filter(test_case::Column::ExecutionId.eq(&test_id))
            .order_by_asc(test_case::Column::SuiteId)
            .order_by_asc(test_case::Column::Id)
            .all(&transaction)
            .await?;
        // Stable order matching SQL jsonb_agg ORDER BY suiteId, id.
        cases.sort_by(|a, b| a.suite_id.cmp(&b.suite_id).then(a.id.cmp(&b.id)));

        let created = created_execution_json(&created_model, &cases);
        transaction.commit().await?;
        Ok(created)
    }

    /// PURPOSE: Implement `test_execution_build_version_inner` domain/persistence logic.
    /// USE: Called as `test_execution_build_version_inner` from handlers, repository impls, or sibling helpers.
    pub(super) async fn test_execution_build_version_inner(
        &self,
        test_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_lifecycle::creation",
            "test_execution_build_version_inner",
        );
        Ok(test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(test_id))
            .select_only()
            .column(test_execution::Column::BuildVersion)
            .into_tuple::<Option<String>>()
            .one(self.orm())
            .await?
            .flatten())
    }

    /// PURPOSE: Implement `executions_by_build_inner` domain/persistence logic.
    /// USE: Called as `executions_by_build_inner` from handlers, repository impls, or sibling helpers.
    pub(super) async fn executions_by_build_inner(
        &self,
        build_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_lifecycle::creation",
            "executions_by_build_inner",
        );
        let total = test_execution::Entity::find()
            .filter(test_execution::Column::BuildId.eq(build_id))
            .count(self.orm())
            .await? as i64;

        let execution_rows = test_execution::Entity::find()
            .filter(test_execution::Column::BuildId.eq(build_id))
            .order_by_desc(test_execution::Column::CreatedAt)
            .limit(limit as u64)
            .offset(offset as u64)
            .all(self.orm())
            .await?;

        let mut executions = Vec::with_capacity(execution_rows.len());
        for execution in execution_rows {
            let cases = test_case::Entity::find()
                .filter(test_case::Column::ExecutionId.eq(&execution.test_id))
                .order_by_asc(test_case::Column::Id)
                .all(self.orm())
                .await?;
            executions.push(execution_with_cases_json(&execution, &cases));
        }

        Ok(serde_json::json!({
            "limit": limit,
            "offset": offset,
            "total": total,
            "executions": executions
        }))
    }

    /// PURPOSE: Implement `test_results_inner` domain/persistence logic.
    /// USE: Called as `test_results_inner` from handlers, repository impls, or sibling helpers.
    pub(super) async fn test_results_inner(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_lifecycle::creation",
            "test_results_inner",
        );
        let Some(execution) = test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(test_id))
            .one(self.orm())
            .await?
        else {
            return Ok(None);
        };
        let cases = test_case::Entity::find()
            .filter(test_case::Column::ExecutionId.eq(test_id))
            .order_by_asc(test_case::Column::Id)
            .all(self.orm())
            .await?;
        Ok(Some(serde_json::json!({
            "status": test_status_str(execution.status.as_ref()),
            "testCases": cases.iter().map(test_case_json).collect::<Vec<_>>(),
        })))
    }
}
