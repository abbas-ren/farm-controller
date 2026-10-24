use super::*;
use super::json_models::{
    device_json, duration_seconds, test_case_json, test_execution_json, test_status_str,
};

use crate::persistence::{
    entity::{device, test_case, test_execution},
    orm_util,
};
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait,
    sea_query::{Expr, extension::postgres::PgExpr},
};

fn execution_list_base(
    user_id: &str,
    search: &str,
) -> sea_orm::Select<test_execution::Entity> {
    let mut select = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id));
    if !search.is_empty() {
        let pattern = format!("%{search}%");
        select = select
            .join(
                JoinType::LeftJoin,
                test_execution::Relation::Device.def(),
            )
            .filter(Expr::col(device::Column::DeviceType).ilike(pattern));
    }
    select
}

#[async_trait]
impl ExecutionQueryRepository for PostgresDeviceRepository {
    async fn list_test_executions(
        &self,
        user_id: &str,
        query: &ExecutionListQuery,
    ) -> Result<ExecutionList, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_queries",
            "list_test_executions",
        );
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(10).max(1);
        let search = query.search.as_deref().unwrap_or_default();
        let sort_by = query
            .sort_by
            .as_deref()
            .filter(|value| {
                matches!(
                    *value,
                    "createdAt" | "status" | "testPlanName" | "buildVersion" | "deviceType"
                )
            })
            .unwrap_or("createdAt");
        let descending = query.desc.as_deref().unwrap_or("true") == "true";

        let total = execution_list_base(user_id, search)
            .count(self.orm())
            .await? as i64;

        let mut ordered = if search.is_empty() {
            test_execution::Entity::find()
                .filter(test_execution::Column::CreatedBy.eq(user_id))
                .find_also_related(device::Entity)
        } else {
            execution_list_base(user_id, search).find_also_related(device::Entity)
        };
        ordered = match (sort_by, descending) {
            ("status", true) => ordered.order_by_desc(test_execution::Column::Status),
            ("status", false) => ordered.order_by_asc(test_execution::Column::Status),
            ("testPlanName", true) => ordered.order_by_desc(test_execution::Column::TestPlanName),
            ("testPlanName", false) => ordered.order_by_asc(test_execution::Column::TestPlanName),
            ("buildVersion", true) => ordered.order_by_desc(test_execution::Column::BuildVersion),
            ("buildVersion", false) => ordered.order_by_asc(test_execution::Column::BuildVersion),
            ("deviceType", true) => ordered.order_by_desc(device::Column::DeviceType),
            ("deviceType", false) => ordered.order_by_asc(device::Column::DeviceType),
            (_, true) => ordered.order_by_desc(test_execution::Column::CreatedAt),
            (_, false) => ordered.order_by_asc(test_execution::Column::CreatedAt),
        };
        let rows = ordered
            .limit(limit as u64)
            .offset(((page - 1) * limit) as u64)
            .all(self.orm())
            .await?;

        let mut data = Vec::with_capacity(rows.len());
        for (execution, device_model) in rows {
            let cases = test_case::Entity::find()
                .filter(test_case::Column::ExecutionId.eq(&execution.test_id))
                .all(self.orm())
                .await?;
            let total_cases = cases.len() as i64;
            let passed = cases
                .iter()
                .filter(|c| matches!(c.result, Some(test_case::TestResult::Pass)))
                .count() as i64;
            let failed = cases
                .iter()
                .filter(|c| matches!(c.result, Some(test_case::TestResult::Fail)))
                .count() as i64;
            data.push(serde_json::json!({
                "testId": execution.test_id,
                "durationSeconds": duration_seconds(execution.started_at, execution.ended_at),
                "total": total_cases,
                "passed": passed,
                "failed": failed,
                "buildId": execution.build_id,
                "buildVersion": execution.build_version,
                "deviceName": device_model.as_ref().and_then(|d| d.device_type.clone()).unwrap_or_default(),
                "deviceId": execution.device_id.clone().unwrap_or_default(),
                "status": test_status_str(execution.status.as_ref()),
                "testCases": {},
                "testPlanName": execution.test_plan_name,
                "logs": [],
                "testCycleId": execution.test_cycle_id.clone().unwrap_or_default(),
                "createdAt": execution.created_at,
                "rtosLogPath": execution.rtos_log_path,
            }));
        }

        Ok(ExecutionList {
            data,
            total: total as u64,
            current_page: page as u64,
            total_pages: ((total + limit - 1) / limit).max(0) as u64,
        })
    }

    async fn test_execution_by_device(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_queries",
            "test_execution_by_device",
        );
        Ok(test_execution::Entity::find()
            .filter(test_execution::Column::DeviceId.eq(device_id))
            .filter(test_execution::Column::CreatedBy.eq(user_id))
            .order_by_desc(test_execution::Column::UpdatedAt)
            .one(self.orm())
            .await?
            .map(|model| test_execution_json(&model)))
    }

    async fn in_progress_test_executions(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_queries",
            "in_progress_test_executions",
        );
        Ok(test_execution::Entity::find()
            .filter(test_execution::Column::CreatedBy.eq(user_id))
            .filter(test_execution::Column::Status.is_in([
                test_execution::TestStatus::InProgress,
                test_execution::TestStatus::NotExecuted,
                test_execution::TestStatus::Queued,
            ]))
            .all(self.orm())
            .await?
            .into_iter()
            .map(|model| {
                serde_json::json!({
                    "deviceId": model.device_id,
                    "status": test_status_str(model.status.as_ref()),
                    "testId": model.test_id,
                })
            })
            .collect())
    }

    async fn single_test_execution(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_queries",
            "single_test_execution",
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
            .await?
            .iter()
            .map(test_case_json)
            .collect::<Vec<_>>();
        let mut value = test_execution_json(&execution);
        value["testCases"] = serde_json::Value::Array(cases);
        Ok(Some(value))
    }

    async fn execution_case_ids(
        &self,
        test_id: &str,
    ) -> Result<Option<Vec<String>>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_queries",
            "execution_case_ids",
        );
        let Some(execution) = test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(test_id))
            .one(self.orm())
            .await?
        else {
            return Ok(None);
        };
        if matches!(
            execution.status,
            Some(test_execution::TestStatus::Cancelled)
                | Some(test_execution::TestStatus::Failed)
        ) {
            return Ok(None);
        }
        Ok(Some(
            test_case::Entity::find()
                .filter(test_case::Column::ExecutionId.eq(test_id))
                .order_by_asc(test_case::Column::SuiteId)
                .order_by_asc(test_case::Column::Id)
                .all(self.orm())
                .await?
                .into_iter()
                .map(|c| c.test_case_id.to_string())
                .collect(),
        ))
    }

    async fn test_execution(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_queries",
            "test_execution",
        );
        let mut select = test_execution::Entity::find()
            .filter(test_execution::Column::CreatedBy.eq(user_id));
        if execution_id == "latest" {
            select = select.filter(test_execution::Column::Status.is_in([
                test_execution::TestStatus::NotExecuted,
                test_execution::TestStatus::Queued,
                test_execution::TestStatus::InProgress,
            ]));
        } else {
            select = select.filter(test_execution::Column::TestId.eq(execution_id));
        }
        let Some((execution, device_model)) = select
            .find_also_related(device::Entity)
            .order_by_desc(test_execution::Column::CreatedAt)
            .one(self.orm())
            .await?
        else {
            return Ok(None);
        };
        let mut value = test_execution_json(&execution);
        value["Device"] = device_model
            .map(|d| device_json(&d))
            .unwrap_or(serde_json::Value::Null);
        Ok(Some(value))
    }

    async fn test_execution_summary(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_queries",
            "test_execution_summary",
        );
        let Some((execution, device_model)) = test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(execution_id))
            .find_also_related(device::Entity)
            .one(self.orm())
            .await?
        else {
            return Ok(None);
        };
        let cases = test_case::Entity::find()
            .filter(test_case::Column::ExecutionId.eq(execution_id))
            .order_by_asc(test_case::Column::Id)
            .all(self.orm())
            .await?;
        let total_cases = cases.len() as i64;
        let passed = cases
            .iter()
            .filter(|c| matches!(c.result, Some(test_case::TestResult::Pass)))
            .count() as i64;
        let failed = cases
            .iter()
            .filter(|c| matches!(c.result, Some(test_case::TestResult::Fail)))
            .count() as i64;
        let device_type = device_model
            .as_ref()
            .and_then(|d| d.device_type.clone())
            .unwrap_or_default();
        let mut summary = serde_json::json!({
            "testId": execution.test_id,
            "durationSeconds": duration_seconds(execution.started_at, execution.ended_at),
            "total": total_cases,
            "passed": passed,
            "failed": failed,
            "buildId": execution.build_id,
            "buildVersion": execution.build_version,
            "deviceName": device_type,
            "deviceType": device_type,
            "status": test_status_str(execution.status.as_ref()),
            "testCases": cases.iter().map(test_case_json).collect::<Vec<_>>(),
            "testPlanName": execution.test_plan_name,
            "testCycleId": execution.test_cycle_id,
            "createdAt": execution.created_at,
            "rtosLogPath": execution.rtos_log_path,
        });

        #[derive(Debug, sea_orm::FromQueryResult)]
        struct AnalyticsRow {
            value: serde_json::Value,
        }
        let analytics = orm_util::query_one::<_, AnalyticsRow>(
            self.orm(),
            r#"WITH eligible AS (
                     SELECT te."createdAt", tc.result::text AS result
                     FROM test_executions te
                     JOIN testcase tc ON tc."executionId" = te."testId"
                     WHERE te."createdBy" = $1
                         AND te.status::text NOT IN ('cancelled', 'queued', 'not_executed', 'failed')
                 )
                 SELECT jsonb_build_object(
                     'allTime', jsonb_build_object(
                         'total', count(*), 'passed', count(*) FILTER (WHERE result = 'PASS'),
                         'failed', count(*) FILTER (WHERE result = 'FAIL'),
                         'inProgress', count(*) FILTER (WHERE result IS NULL OR result NOT IN ('PASS', 'FAIL'))),
                     'currentWeek', jsonb_build_object(
                         'week', to_char(date_trunc('week', now()), 'DD Mon') || ' - ' || to_char(date_trunc('week', now()) + interval '6 days', 'DD Mon'),
                         'total', count(*) FILTER (WHERE "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week'),
                         'passed', count(*) FILTER (WHERE result = 'PASS' AND "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week'),
                         'failed', count(*) FILTER (WHERE result = 'FAIL' AND "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week'),
                         'inProgress', count(*) FILTER (WHERE (result IS NULL OR result NOT IN ('PASS', 'FAIL')) AND "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week')),
                     'currentMonth', jsonb_build_object(
                         'month', to_char(date_trunc('month', now()), 'Mon YYYY'),
                         'total', count(*) FILTER (WHERE "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month'),
                         'passed', count(*) FILTER (WHERE result = 'PASS' AND "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month'),
                         'failed', count(*) FILTER (WHERE result = 'FAIL' AND "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month'),
                         'inProgress', count(*) FILTER (WHERE (result IS NULL OR result NOT IN ('PASS', 'FAIL')) AND "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month'))) AS value
                 FROM eligible"#,
            vec![user_id.into()],
        )
        .await?
        .map(|row| row.value)
        .unwrap_or_else(|| serde_json::json!({}));
        summary["analytics"] = analytics;
        Ok(Some(summary))
    }

    async fn test_export_rows(
        &self,
        user_id: &str,
    ) -> Result<Vec<TestExportRow>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::execution_queries",
            "test_export_rows",
        );
        test_export::rows(self.orm(), user_id).await
    }
}
