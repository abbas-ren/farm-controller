//! `ExecutionQueryRepository` impl: list/detail/summary/export queries.
//!
//! Ports Node `TestService` execution listing and analytics helpers.

use super::*;
use super::json_models::{
    device_json, duration_seconds, test_case_json, test_execution_json, test_status_str,
};

use crate::persistence::entity::{device, test_case, test_execution};
use chrono::{Datelike, Duration, Utc};
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait,
    sea_query::{Expr, extension::postgres::PgExpr},
};

/// PURPOSE: Implement `pg_week_start` domain/persistence logic.
/// USE: Called as `pg_week_start` from handlers, repository impls, or sibling helpers.
fn pg_week_start(now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    // PostgreSQL date_trunc('week') uses ISO weeks starting Monday 00:00.
    let days_from_monday = now.weekday().num_days_from_monday() as i64;
    (now - Duration::days(days_from_monday))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
}

/// PURPOSE: Implement `pg_month_start` domain/persistence logic.
/// USE: Called as `pg_month_start` from handlers, repository impls, or sibling helpers.
fn pg_month_start(now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    now.date_naive()
        .with_day(1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
}

/// PURPOSE: Implement `execution_analytics` domain/persistence logic.
/// USE: Called as `execution_analytics` from handlers, repository impls, or sibling helpers.
async fn execution_analytics(
    orm: &sea_orm::DatabaseConnection,
    user_id: &str,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let now = Utc::now();
    let week_start = pg_week_start(now);
    let week_end = week_start + Duration::days(7);
    let month_start = pg_month_start(now);
    let month_end = if month_start.month() == 12 {
        month_start
            .with_year(month_start.year() + 1)
            .and_then(|d| d.with_month(1))
            .unwrap_or(month_start)
    } else {
        month_start
            .with_month(month_start.month() + 1)
            .unwrap_or(month_start)
    };

    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id))
        .filter(
            test_execution::Column::Status.is_not_in([
                test_execution::TestStatus::Cancelled,
                test_execution::TestStatus::Queued,
                test_execution::TestStatus::NotExecuted,
                test_execution::TestStatus::Failed,
            ]),
        )
        .all(orm)
        .await?;

    let mut all_total = 0i64;
    let mut all_passed = 0i64;
    let mut all_failed = 0i64;
    let mut all_in_progress = 0i64;
    let mut week_total = 0i64;
    let mut week_passed = 0i64;
    let mut week_failed = 0i64;
    let mut week_in_progress = 0i64;
    let mut month_total = 0i64;
    let mut month_passed = 0i64;
    let mut month_failed = 0i64;
    let mut month_in_progress = 0i64;

    for execution in executions {
        let cases = test_case::Entity::find()
            .filter(test_case::Column::ExecutionId.eq(&execution.test_id))
            .all(orm)
            .await?;
        let in_week = execution.created_at >= week_start && execution.created_at < week_end;
        let in_month = execution.created_at >= month_start && execution.created_at < month_end;
        for case in cases {
            let (is_pass, is_fail, is_progress) = match case.result {
                Some(test_case::TestResult::Pass) => (true, false, false),
                Some(test_case::TestResult::Fail) => (false, true, false),
                None => (false, false, true),
            };
            all_total += 1;
            if is_pass {
                all_passed += 1;
            } else if is_fail {
                all_failed += 1;
            } else if is_progress {
                all_in_progress += 1;
            }
            if in_week {
                week_total += 1;
                if is_pass {
                    week_passed += 1;
                } else if is_fail {
                    week_failed += 1;
                } else if is_progress {
                    week_in_progress += 1;
                }
            }
            if in_month {
                month_total += 1;
                if is_pass {
                    month_passed += 1;
                } else if is_fail {
                    month_failed += 1;
                } else if is_progress {
                    month_in_progress += 1;
                }
            }
        }
    }

    let week_label = format!(
        "{} - {}",
        week_start.format("%d %b"),
        (week_start + Duration::days(6)).format("%d %b")
    );
    let month_label = month_start.format("%b %Y").to_string();

    Ok(serde_json::json!({
        "allTime": {
            "total": all_total,
            "passed": all_passed,
            "failed": all_failed,
            "inProgress": all_in_progress,
        },
        "currentWeek": {
            "week": week_label,
            "total": week_total,
            "passed": week_passed,
            "failed": week_failed,
            "inProgress": week_in_progress,
        },
        "currentMonth": {
            "month": month_label,
            "total": month_total,
            "passed": month_passed,
            "failed": month_failed,
            "inProgress": month_in_progress,
        },
    }))
}

/// PURPOSE: Build SeaORM filter/order clauses for `execution list base`.
/// USE: Shared by count and page queries.
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
    /// PURPOSE: List/filter `list test executions` rows for the API.
    /// USE: HTTP list handlers and repository trait methods.
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

    /// PURPOSE: Implement `test_execution_by_device` domain/persistence logic.
    /// USE: Called as `test_execution_by_device` from handlers, repository impls, or sibling helpers.
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

    /// PURPOSE: Implement `in_progress_test_executions` domain/persistence logic.
    /// USE: Called as `in_progress_test_executions` from handlers, repository impls, or sibling helpers.
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

    /// PURPOSE: Implement `single_test_execution` domain/persistence logic.
    /// USE: Called as `single_test_execution` from handlers, repository impls, or sibling helpers.
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

    /// PURPOSE: Implement `execution_case_ids` domain/persistence logic.
    /// USE: Called as `execution_case_ids` from handlers, repository impls, or sibling helpers.
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

    /// PURPOSE: Implement `test_execution` domain/persistence logic.
    /// USE: Called as `test_execution` from handlers, repository impls, or sibling helpers.
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

    /// PURPOSE: Implement `test_execution_summary` domain/persistence logic.
    /// USE: Called as `test_execution_summary` from handlers, repository impls, or sibling helpers.
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

        summary["analytics"] = execution_analytics(self.orm(), user_id).await?;
        Ok(Some(summary))
    }

    /// PURPOSE: Implement `test_export_rows` domain/persistence logic.
    /// USE: Called as `test_export_rows` from handlers, repository impls, or sibling helpers.
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
