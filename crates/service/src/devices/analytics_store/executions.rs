//! Execution-centric analytics (recent, daily, plans, in-progress).
//!
//! Used by analytics HTTP handlers for test dashboards. Ports Node
//! `AnalyticsService` execution analytics endpoints.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, Utc};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use crate::persistence::entity::{test_case, test_execution};

use super::super::error::DeviceRepositoryError;

#[derive(Debug)]
/// Domain/DTO type `TestCaseAnalyticsRow` used across device stores and handlers.
struct TestCaseAnalyticsRow {
    created_at: DateTime<Utc>,
    result: String,
}

/// PURPOSE: Implement `recent_executions` domain/persistence logic.
/// USE: Called as `recent_executions` from handlers, repository impls, or sibling helpers.
pub(crate) async fn recent_executions(
    orm: &DatabaseConnection,
    user_id: &str,
    count: i64,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "recent_executions",
    );
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id))
        .order_by_desc(test_execution::Column::CreatedAt)
        .limit(count as u64)
        .all(orm)
        .await?;
    let counts = case_counts_by_execution(orm, &execution_ids(&executions)).await?;
    Ok(executions
        .into_iter()
        .map(|te| {
            let total = counts.get(&te.test_id).copied().unwrap_or(0);
            serde_json::json!({
                "testId": te.test_id,
                "buildVersion": te.build_version,
                "status": status_str(te.status.as_ref()),
                "testPlanName": te.test_plan_name,
                "createdAt": te.created_at,
                "deviceType": te.device_type,
                "startedAt": te.started_at,
                "endedAt": te.ended_at,
                "totalTestCases": total,
            })
        })
        .collect())
}

/// PURPOSE: Fetch a single record for `execution by id`.
/// USE: Detail handlers and repository lookups.
pub(crate) async fn execution_by_id(
    orm: &DatabaseConnection,
    test_id: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "execution_by_id",
    );
    let Some(te) = test_execution::Entity::find()
        .filter(test_execution::Column::TestId.eq(test_id))
        .order_by_desc(test_execution::Column::CreatedAt)
        .one(orm)
        .await?
    else {
        return Ok(None);
    };
    let total = test_case::Entity::find()
        .filter(test_case::Column::ExecutionId.eq(&te.test_id))
        .count(orm)
        .await? as i64;
    Ok(Some(serde_json::json!({
        "testId": te.test_id,
        "buildVersion": te.build_version,
        "status": status_str(te.status.as_ref()),
        "testPlanName": te.test_plan_name,
        "createdAt": te.created_at,
        "totalTestCases": total,
    })))
}

/// PURPOSE: Implement `execution_daily` domain/persistence logic.
/// USE: Called as `execution_daily` from handlers, repository impls, or sibling helpers.
pub(crate) async fn execution_daily(
    orm: &DatabaseConnection,
    user_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "execution_daily",
    );
    let start = from
        .and_hms_opt(0, 0, 0)
        .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
        .unwrap_or_else(Utc::now);
    let end_exclusive = to
        .checked_add_days(Days::new(1))
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
        .unwrap_or_else(Utc::now);
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id))
        .filter(
            test_execution::Column::Status.is_in([
                test_execution::TestStatus::Completed,
                test_execution::TestStatus::InProgress,
            ]),
        )
        .filter(test_execution::Column::CreatedAt.gte(start))
        .filter(test_execution::Column::CreatedAt.lt(end_exclusive))
        .all(orm)
        .await?;
    let case_rows = load_cases_for_executions(orm, &execution_ids(&executions)).await?;
    let now = Utc::now();
    let mut by_day: HashMap<NaiveDate, DayAgg> = HashMap::new();
    let mut day = from;
    while day <= to {
        by_day.insert(day, DayAgg::default());
        match day.checked_add_days(Days::new(1)) {
            Some(next) => day = next,
            None => break,
        }
    }
    for te in &executions {
        let day = te.created_at.date_naive();
        let entry = by_day.entry(day).or_default();
        entry.total_executions += 1;
        let start_at = te.started_at.unwrap_or(te.created_at);
        let end_at = te.ended_at.unwrap_or(now);
        entry.total_duration_seconds += (end_at - start_at).num_seconds().max(0);
        if let Some(cases) = case_rows.get(&te.test_id) {
            for result in cases {
                match result.as_str() {
                    "PASS" => entry.passed_test_cases += 1,
                    "FAIL" => entry.failed_test_cases += 1,
                    _ => {}
                }
            }
        }
    }
    let mut days: Vec<_> = by_day.into_iter().collect();
    days.sort_by_key(|(day, _)| *day);
    Ok(days
        .into_iter()
        .map(|(day, agg)| {
            serde_json::json!({
                "date": day.format("%Y-%m-%d").to_string(),
                "totalExecutions": agg.total_executions,
                "passedTestCases": agg.passed_test_cases,
                "failedTestCases": agg.failed_test_cases,
                "totalDurationSeconds": agg.total_duration_seconds,
            })
        })
        .collect())
}

/// PURPOSE: Implement `test_execution_analytics` domain/persistence logic.
/// USE: Called as `test_execution_analytics` from handlers, repository impls, or sibling helpers.
pub(crate) async fn test_execution_analytics(
    orm: &DatabaseConnection,
    user_id: &str,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "test_execution_analytics",
    );
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id))
        .filter(test_execution::Column::Status.is_not_in([
            test_execution::TestStatus::Cancelled,
            test_execution::TestStatus::Queued,
            test_execution::TestStatus::NotExecuted,
            test_execution::TestStatus::Failed,
        ]))
        .all(orm)
        .await?;
    let cases = load_cases_for_executions(orm, &execution_ids(&executions)).await?;
    let mut rows = Vec::new();
    for te in executions {
        if let Some(case_results) = cases.get(&te.test_id) {
            for result in case_results {
                if result.is_empty() {
                    continue;
                }
                rows.push(TestCaseAnalyticsRow {
                    created_at: te.created_at,
                    result: result.clone(),
                });
            }
        }
    }
    let today = Utc::now().date_naive();
    let current_week = today
        .checked_sub_days(Days::new(u64::from(today.weekday().num_days_from_sunday())))
        .unwrap_or(today);
    let current_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let periodic_start = current_month
        .checked_sub_months(Months::new(4))
        .unwrap_or(current_month);
    let all_time = aggregate_test_cases(rows.iter());
    let mut weekly = Vec::with_capacity(6);
    let mut monthly = Vec::with_capacity(6);
    for offset in (0..6).rev() {
        let week_start = current_week
            .checked_sub_days(Days::new(offset * 7))
            .unwrap_or(current_week);
        let week_end = week_start
            .checked_add_days(Days::new(6))
            .unwrap_or(week_start);
        let mut counts = aggregate_test_cases(rows.iter().filter(|row| {
            let day = row.created_at.date_naive();
            day >= periodic_start && day >= week_start && day <= week_end
        }));
        counts["week"] = serde_json::json!(format!(
            "{} - {}",
            week_start.format("%d %b"),
            week_end.format("%d %b")
        ));
        weekly.push(counts);

        let month_start = current_month
            .checked_sub_months(Months::new(offset as u32))
            .unwrap_or(current_month);
        let month_end = month_start
            .checked_add_months(Months::new(1))
            .and_then(|next| next.checked_sub_days(Days::new(1)))
            .unwrap_or(month_start);
        let mut counts = aggregate_test_cases(rows.iter().filter(|row| {
            let day = row.created_at.date_naive();
            day >= periodic_start && day >= month_start && day <= month_end
        }));
        counts["month"] = serde_json::json!(month_start.format("%b %Y").to_string());
        monthly.push(counts);
    }
    Ok(serde_json::json!({
        "allTime": all_time,
        "weekly": weekly,
        "monthly": monthly,
    }))
}

/// PURPOSE: Implement `in_progress_test_analytics` domain/persistence logic.
/// USE: Called as `in_progress_test_analytics` from handlers, repository impls, or sibling helpers.
pub(crate) async fn in_progress_test_analytics(
    orm: &DatabaseConnection,
    user_id: &str,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "in_progress_test_analytics",
    );
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id))
        .filter(test_execution::Column::Status.eq(test_execution::TestStatus::InProgress))
        .order_by_desc(test_execution::Column::CreatedAt)
        .all(orm)
        .await?;
    let cases = load_case_models(orm, &execution_ids(&executions)).await?;
    Ok(executions
        .into_iter()
        .map(|te| {
            let case_list = cases.get(&te.test_id);
            let total = case_list.map(|list| list.len() as i64).unwrap_or(0);
            let executed = case_list
                .map(|list| {
                    list.iter()
                        .filter(|case| {
                            matches!(
                                case.result,
                                Some(test_case::TestResult::Pass) | Some(test_case::TestResult::Fail)
                            )
                        })
                        .count() as i64
                })
                .unwrap_or(0);
            serde_json::json!({
                "id": te.id,
                "testPlanName": te.test_plan_name,
                "createdAt": te.created_at,
                "startedAt": te.started_at,
                "endedAt": te.ended_at,
                "status": status_str(te.status.as_ref()),
                "total": total,
                "executed": executed,
            })
        })
        .collect())
}

/// PURPOSE: Implement `test_plan_summary` domain/persistence logic.
/// USE: Called as `test_plan_summary` from handlers, repository impls, or sibling helpers.
pub(crate) async fn test_plan_summary(
    orm: &DatabaseConnection,
    user_id: &str,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "test_plan_summary",
    );
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id))
        .all(orm)
        .await?;
    let cases = load_cases_for_executions(orm, &execution_ids(&executions)).await?;
    let mut total = 0_i64;
    let mut completed = 0_i64;
    let mut failed = 0_i64;
    let mut cancelled = 0_i64;
    let mut in_progress = 0_i64;
    for te in executions {
        total += 1;
        let has_fail = cases
            .get(&te.test_id)
            .map(|list| list.iter().any(|result| result == "FAIL"))
            .unwrap_or(false);
        match te.status {
            Some(test_execution::TestStatus::Completed) if !has_fail => completed += 1,
            Some(test_execution::TestStatus::Failed)
            | Some(test_execution::TestStatus::Completed) => failed += 1,
            Some(test_execution::TestStatus::Cancelled) => cancelled += 1,
            _ => in_progress += 1,
        }
    }
    Ok(serde_json::json!({
        "total": total,
        "completed": completed,
        "failed": failed,
        "cancelled": cancelled,
        "inProgress": in_progress,
    }))
}

/// PURPOSE: Implement `daily_test_summary` domain/persistence logic.
/// USE: Called as `daily_test_summary` from handlers, repository impls, or sibling helpers.
pub(crate) async fn daily_test_summary(
    orm: &DatabaseConnection,
    user_id: &str,
    days: i64,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "daily_test_summary",
    );
    let today = Utc::now().date_naive();
    let from = today
        .checked_sub_days(Days::new((days.max(1) as u64).saturating_sub(1)))
        .unwrap_or(today);
    let start = from
        .and_hms_opt(0, 0, 0)
        .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
        .unwrap_or_else(Utc::now);
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id))
        .filter(test_execution::Column::Status.ne(test_execution::TestStatus::Cancelled))
        .filter(test_execution::Column::CreatedAt.gte(start))
        .all(orm)
        .await?;
    let cases = load_cases_for_executions(orm, &execution_ids(&executions)).await?;
    let now = Utc::now();
    let mut by_day: HashMap<NaiveDate, DailySummaryAgg> = HashMap::new();
    let mut day = from;
    while day <= today {
        by_day.insert(day, DailySummaryAgg::default());
        let next = day.checked_add_days(Days::new(1)).unwrap_or(day);
        if next == day {
            break;
        }
        day = next;
    }
    for te in &executions {
        let day = te.created_at.date_naive();
        let entry = by_day.entry(day).or_default();
        let start_at = te.started_at.unwrap_or(te.created_at);
        let end_at = te.ended_at.unwrap_or(now);
        entry.total_execution_seconds += (end_at - start_at).num_seconds().max(0);
        if let Some(case_list) = cases.get(&te.test_id) {
            for result in case_list {
                match result.as_str() {
                    "PASS" => entry.passed += 1,
                    "FAIL" => entry.failed += 1,
                    _ => {}
                }
            }
        }
    }
    let mut days: Vec<_> = by_day.into_iter().collect();
    days.sort_by_key(|(day, _)| *day);
    Ok(days
        .into_iter()
        .map(|(day, agg)| {
            serde_json::json!({
                "date": day.format("%Y-%m-%d").to_string(),
                "totalExecutionSeconds": agg.total_execution_seconds,
                "passed": agg.passed,
                "failed": agg.failed,
            })
        })
        .collect())
}

#[derive(Default)]
/// Domain/DTO type `DayAgg` used across device stores and handlers.
struct DayAgg {
    total_executions: i64,
    passed_test_cases: i64,
    failed_test_cases: i64,
    total_duration_seconds: i64,
}

#[derive(Default)]
/// Domain/DTO type `DailySummaryAgg` used across device stores and handlers.
struct DailySummaryAgg {
    total_execution_seconds: i64,
    passed: i64,
    failed: i64,
}

/// PURPOSE: Implement `execution_ids` domain/persistence logic.
/// USE: Called as `execution_ids` from handlers, repository impls, or sibling helpers.
fn execution_ids(executions: &[test_execution::Model]) -> Vec<String> {
    executions
        .iter()
        .map(|execution| execution.test_id.clone())
        .collect()
}

/// PURPOSE: Implement `case_counts_by_execution` domain/persistence logic.
/// USE: Called as `case_counts_by_execution` from handlers, repository impls, or sibling helpers.
async fn case_counts_by_execution(
    orm: &DatabaseConnection,
    ids: &[String],
) -> Result<HashMap<String, i64>, DeviceRepositoryError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let cases = test_case::Entity::find()
        .filter(test_case::Column::ExecutionId.is_in(ids.to_vec()))
        .all(orm)
        .await?;
    let mut counts = HashMap::new();
    for case in cases {
        if let Some(execution_id) = case.execution_id {
            *counts.entry(execution_id).or_insert(0) += 1;
        }
    }
    Ok(counts)
}

/// PURPOSE: Load or save `load cases for executions`.
/// USE: Internal store helpers during multi-step transactions.
async fn load_cases_for_executions(
    orm: &DatabaseConnection,
    ids: &[String],
) -> Result<HashMap<String, Vec<String>>, DeviceRepositoryError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let cases = test_case::Entity::find()
        .filter(test_case::Column::ExecutionId.is_in(ids.to_vec()))
        .all(orm)
        .await?;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for case in cases {
        let Some(execution_id) = case.execution_id else {
            continue;
        };
        let result = match case.result {
            Some(test_case::TestResult::Pass) => "PASS".to_owned(),
            Some(test_case::TestResult::Fail) => "FAIL".to_owned(),
            None => String::new(),
        };
        map.entry(execution_id).or_default().push(result);
    }
    Ok(map)
}

/// PURPOSE: Load or save `load case models`.
/// USE: Internal store helpers during multi-step transactions.
async fn load_case_models(
    orm: &DatabaseConnection,
    ids: &[String],
) -> Result<HashMap<String, Vec<test_case::Model>>, DeviceRepositoryError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let cases = test_case::Entity::find()
        .filter(test_case::Column::ExecutionId.is_in(ids.to_vec()))
        .all(orm)
        .await?;
    let mut map: HashMap<String, Vec<test_case::Model>> = HashMap::new();
    for case in cases {
        if let Some(execution_id) = case.execution_id.clone() {
            map.entry(execution_id).or_default().push(case);
        }
    }
    Ok(map)
}

/// PURPOSE: Serialize `status str` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
fn status_str(status: Option<&test_execution::TestStatus>) -> &'static str {
    match status {
        Some(test_execution::TestStatus::NotExecuted) => "not_executed",
        Some(test_execution::TestStatus::InProgress) => "in_progress",
        Some(test_execution::TestStatus::Completed) => "completed",
        Some(test_execution::TestStatus::Queued) => "queued",
        Some(test_execution::TestStatus::Cancelled) => "cancelled",
        Some(test_execution::TestStatus::Failed) => "failed",
        None => "",
    }
}

/// PURPOSE: Implement `aggregate_test_cases` domain/persistence logic.
/// USE: Called as `aggregate_test_cases` from handlers, repository impls, or sibling helpers.
fn aggregate_test_cases<'a>(
    rows: impl Iterator<Item = &'a TestCaseAnalyticsRow>,
) -> serde_json::Value {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "aggregate_test_cases",
    );
    let mut total = 0_i64;
    let mut passed = 0_i64;
    let mut failed = 0_i64;
    let mut in_progress = 0_i64;
    for row in rows {
        total += 1;
        match row.result.as_str() {
            "PASS" => passed += 1,
            "FAIL" => failed += 1,
            _ => in_progress += 1,
        }
    }
    serde_json::json!({
        "total": total,
        "passed": passed,
        "failed": failed,
        "inProgress": in_progress,
    })
}
