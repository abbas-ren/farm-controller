//! Build comparison and performance analytics queries.
//!
//! Used by analytics HTTP handlers for build dashboards. Ports Node
//! `AnalyticsService` build comparison / performance aggregations.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::persistence::entity::{device_action_queue, release, test_case, test_execution};

use super::super::error::DeviceRepositoryError;

/// PURPOSE: Implement `build_comparisons` domain/persistence logic.
/// USE: Called as `build_comparisons` from handlers, repository impls, or sibling helpers.
pub(crate) async fn build_comparisons(
    orm: &DatabaseConnection,
    user_id: &str,
    count: i64,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::builds",
        "build_comparisons",
    );
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id))
        .filter(test_execution::Column::Status.eq(test_execution::TestStatus::Completed))
        .filter(test_execution::Column::BuildId.is_not_null())
        .all(orm)
        .await?;
    let build_ids: HashSet<String> = executions
        .iter()
        .map(|execution| execution.build_id.clone())
        .collect();
    if build_ids.is_empty() {
        return Ok(Vec::new());
    }
    let releases = load_releases(orm, &build_ids).await?;
    let all_completed = test_execution::Entity::find()
        .filter(test_execution::Column::BuildId.is_in(build_ids.iter().cloned().collect::<Vec<_>>()))
        .filter(test_execution::Column::Status.eq(test_execution::TestStatus::Completed))
        .all(orm)
        .await?;
    let cases = load_cases(orm, &execution_ids(&all_completed)).await?;
    let now = Utc::now();
    let mut metrics: HashMap<String, BuildMetrics> = HashMap::new();
    for execution in &executions {
        let entry = metrics.entry(execution.build_id.clone()).or_default();
        entry.last_used = Some(
            entry
                .last_used
                .map_or(execution.created_at, |previous| previous.max(execution.created_at)),
        );
    }
    for execution in &all_completed {
        let entry = metrics.entry(execution.build_id.clone()).or_default();
        entry.executions += 1;
        let start = execution.started_at.unwrap_or(execution.created_at);
        let end = execution.ended_at.unwrap_or(now);
        entry.duration_ms += ((end - start).num_milliseconds().max(0)) as f64;
        if let Some(case_list) = cases.get(&execution.test_id) {
            for result in case_list {
                entry.total_cases += 1;
                if result == "PASS" {
                    entry.passed_cases += 1;
                }
            }
        }
    }
    let mut rows: Vec<_> = metrics
        .into_iter()
        .filter_map(|(build_id, metric)| {
            let release = releases.get(&build_id)?;
            let average_duration = if metric.executions > 0 {
                round2(metric.duration_ms / metric.executions as f64)
            } else {
                0.0
            };
            let passed_percentage = if metric.total_cases > 0 {
                round2((metric.passed_cases as f64 / metric.total_cases as f64) * 100.0)
            } else {
                0.0
            };
            let pass_ratio = if metric.total_cases > 0 {
                metric.passed_cases as f64 / metric.total_cases as f64
            } else {
                0.0
            };
            Some((
                pass_ratio,
                metric.last_used.unwrap_or_else(Utc::now),
                serde_json::json!({
                    "buildId": build_id,
                    "buildVersion": release.version,
                    "deviceType": release.device_type,
                    "averageDuration": average_duration,
                    "totalTestCases": metric.total_cases,
                    "passedPercentage": passed_percentage,
                }),
            ))
        })
        .collect();
    rows.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))
    });

    Ok(rows
        .into_iter()
        .take(count.max(0) as usize)
        .map(|(_, _, value)| value)
        .collect())
}

/// PURPOSE: Fetch a single record for `build comparison by id`.
/// USE: Detail handlers and repository lookups.
pub(crate) async fn build_comparison_by_id(
    orm: &DatabaseConnection,
    user_id: &str,
    build_id: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::builds",
        "build_comparison_by_id",
    );
    let owned = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id))
        .filter(test_execution::Column::BuildId.eq(build_id))
        .filter(test_execution::Column::Status.eq(test_execution::TestStatus::Completed))
        .one(orm)
        .await?;
    if owned.is_none() {
        return Ok(None);
    }
    Ok(Some(build_comparison_payload(orm, build_id).await?))
}

/// PURPOSE: Implement `build_performance` domain/persistence logic.
/// USE: Called as `build_performance` from handlers, repository impls, or sibling helpers.
pub(crate) async fn build_performance(
    orm: &DatabaseConnection,
    count: i64,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::builds",
        "build_performance",
    );
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::BuildId.is_not_null())
        .all(orm)
        .await?;
    let mut last_used: HashMap<String, DateTime<Utc>> = HashMap::new();
    for execution in &executions {
        last_used
            .entry(execution.build_id.clone())
            .and_modify(|current| {
                if execution.created_at > *current {
                    *current = execution.created_at;
                }
            })
            .or_insert(execution.created_at);
    }
    let mut builds: Vec<_> = last_used.into_iter().collect();
    builds.sort_by(|a, b| b.1.cmp(&a.1));
    builds.truncate(count.max(0) as usize);
    let mut result = Vec::with_capacity(builds.len());
    for (build_id, _) in builds {
        if find_release(orm, &build_id).await?.is_none() {
            continue;
        }
        if let Some(payload) = build_performance_payload(orm, &build_id).await? {
            result.push(payload);
        }
    }
    Ok(result)
}

/// PURPOSE: Fetch a single record for `build performance by id`.
/// USE: Detail handlers and repository lookups.
pub(crate) async fn build_performance_by_id(
    orm: &DatabaseConnection,
    build_id: &str,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::builds",
        "build_performance_by_id",
    );
    Ok(build_performance_payload(orm, build_id)
        .await?
        .unwrap_or_else(|| serde_json::json!({})))
}

/// PURPOSE: Implement `build_comparison_payload` domain/persistence logic.
/// USE: Called as `build_comparison_payload` from handlers, repository impls, or sibling helpers.
async fn build_comparison_payload(
    orm: &DatabaseConnection,
    build_id: &str,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let release = find_release(orm, build_id).await?;
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::BuildId.eq(build_id))
        .filter(test_execution::Column::Status.eq(test_execution::TestStatus::Completed))
        .all(orm)
        .await?;
    let cases = load_cases(orm, &execution_ids(&executions)).await?;
    let now = Utc::now();
    let mut duration_sum = 0.0_f64;
    let mut total_cases = 0_i64;
    let mut passed_cases = 0_i64;
    for execution in &executions {
        let start = execution.started_at.unwrap_or(execution.created_at);
        let end = execution.ended_at.unwrap_or(now);
        duration_sum += (end - start).num_milliseconds().max(0) as f64;
        if let Some(case_list) = cases.get(&execution.test_id) {
            for result in case_list {
                total_cases += 1;
                if result == "PASS" {
                    passed_cases += 1;
                }
            }
        }
    }
    let average_duration = if executions.is_empty() {
        0.0
    } else {
        round2(duration_sum / executions.len() as f64)
    };
    let passed_percentage = if total_cases > 0 {
        round2((passed_cases as f64 / total_cases as f64) * 100.0)
    } else {
        0.0
    };
    let build_version = release
        .as_ref()
        .map(|row| row.version.clone())
        .or_else(|| {
            executions
                .iter()
                .max_by_key(|execution| execution.created_at)
                .and_then(|execution| execution.build_version.clone())
        });
    Ok(serde_json::json!({
        "buildId": build_id,
        "buildVersion": build_version,
        "deviceType": release.as_ref().map(|row| row.device_type.clone()),
        "averageDuration": average_duration,
        "totalTestCases": total_cases,
        "passedPercentage": passed_percentage,
    }))
}

/// PURPOSE: Implement `build_performance_payload` domain/persistence logic.
/// USE: Called as `build_performance_payload` from handlers, repository impls, or sibling helpers.
async fn build_performance_payload(
    orm: &DatabaseConnection,
    build_id: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let release = find_release(orm, build_id).await?;
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::BuildId.eq(build_id))
        .all(orm)
        .await?;
    if release.is_none() && executions.is_empty() {
        return Ok(Some(serde_json::json!({
            "buildId": build_id,
            "buildVersion": serde_json::Value::Null,
            "flagged": false,
            "deviceType": serde_json::Value::Null,
            "status": serde_json::Value::Null,
            "lastTestExecutionId": serde_json::Value::Null,
            "passedTestCaseCount": 0,
            "failedTestCaseCount": 0,
            "passedTestCasePercentage": 0.0,
            "averageExecutionTime": 0.0,
            "uniqueDeviceCount": 0,
            "executionCount": 0,
        })));
    }
    let cases = load_cases(orm, &execution_ids(&executions)).await?;
    let now = Utc::now();
    let mut duration_sum = 0.0_f64;
    let mut passed = 0_i64;
    let mut failed = 0_i64;
    for execution in &executions {
        let start = execution.started_at.unwrap_or(execution.created_at);
        let end = execution.ended_at.unwrap_or(now);
        duration_sum += (end - start).num_milliseconds().max(0) as f64;
        if let Some(case_list) = cases.get(&execution.test_id) {
            for result in case_list {
                match result.as_str() {
                    "PASS" => passed += 1,
                    "FAIL" => failed += 1,
                    _ => {}
                }
            }
        }
    }
    let decided = passed + failed;
    let passed_percentage = if decided > 0 {
        round2((passed as f64 / decided as f64) * 100.0)
    } else {
        0.0
    };
    let average_execution_time = if executions.is_empty() {
        0.0
    } else {
        round2(duration_sum / executions.len() as f64)
    };
    let latest = executions
        .iter()
        .max_by_key(|execution| execution.created_at);
    let release_id = Uuid::parse_str(build_id).ok();
    let unique_device_count = if let Some(release_id) = release_id {
        let devices = device_action_queue::Entity::find()
            .filter(device_action_queue::Column::ReleaseId.eq(release_id))
            .filter(device_action_queue::Column::TargetDeviceId.is_not_null())
            .all(orm)
            .await?;
        devices
            .into_iter()
            .filter_map(|row| row.target_device_id)
            .collect::<HashSet<_>>()
            .len() as i64
    } else {
        0
    };
    let build_version = release
        .as_ref()
        .map(|row| row.version.clone())
        .or_else(|| latest.and_then(|execution| execution.build_version.clone()));
    let status = release
        .as_ref()
        .and_then(|row| row.status.as_ref().map(release_status_str))
        .or_else(|| latest.and_then(|execution| execution.status.as_ref().map(test_status_str)));
    Ok(Some(serde_json::json!({
        "buildId": build_id,
        "buildVersion": build_version,
        "flagged": release.as_ref().map(|row| row.is_faulty).unwrap_or(false),
        "deviceType": release.as_ref().map(|row| row.device_type.clone()),
        "status": status,
        "lastTestExecutionId": latest.map(|execution| execution.test_id.clone()),
        "passedTestCaseCount": passed,
        "failedTestCaseCount": failed,
        "passedTestCasePercentage": passed_percentage,
        "averageExecutionTime": average_execution_time,
        "uniqueDeviceCount": unique_device_count,
        "executionCount": executions.len() as i64,
    })))
}

#[derive(Default)]
/// Domain/DTO type `BuildMetrics` used across device stores and handlers.
struct BuildMetrics {
    last_used: Option<DateTime<Utc>>,
    executions: i64,
    duration_ms: f64,
    total_cases: i64,
    passed_cases: i64,
}

/// PURPOSE: Implement `execution_ids` domain/persistence logic.
/// USE: Called as `execution_ids` from handlers, repository impls, or sibling helpers.
fn execution_ids(executions: &[test_execution::Model]) -> Vec<String> {
    executions
        .iter()
        .map(|execution| execution.test_id.clone())
        .collect()
}

/// PURPOSE: Load or save `load releases`.
/// USE: Internal store helpers during multi-step transactions.
async fn load_releases(
    orm: &DatabaseConnection,
    build_ids: &HashSet<String>,
) -> Result<HashMap<String, release::Model>, DeviceRepositoryError> {
    let mut map = HashMap::new();
    for build_id in build_ids {
        if let Some(release) = find_release(orm, build_id).await? {
            map.insert(build_id.clone(), release);
        }
    }
    Ok(map)
}

/// PURPOSE: Implement `find_release` domain/persistence logic.
/// USE: Called as `find_release` from handlers, repository impls, or sibling helpers.
async fn find_release(
    orm: &DatabaseConnection,
    build_id: &str,
) -> Result<Option<release::Model>, DeviceRepositoryError> {
    let Ok(id) = Uuid::parse_str(build_id) else {
        return Ok(None);
    };
    Ok(release::Entity::find_by_id(id).one(orm).await?)
}

/// PURPOSE: Load or save `load cases`.
/// USE: Internal store helpers during multi-step transactions.
async fn load_cases(
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
            None => continue,
        };
        map.entry(execution_id).or_default().push(result);
    }
    Ok(map)
}

/// PURPOSE: Implement `round2` domain/persistence logic.
/// USE: Called as `round2` from handlers, repository impls, or sibling helpers.
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// PURPOSE: Serialize `release status str` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
fn release_status_str(status: &release::ReleaseTestStatus) -> &'static str {
    match status {
        release::ReleaseTestStatus::Untested => "untested",
        release::ReleaseTestStatus::Testing => "testing",
        release::ReleaseTestStatus::Passed => "passed",
        release::ReleaseTestStatus::Failed => "failed",
    }
}

/// PURPOSE: Serialize `test status str` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
fn test_status_str(status: &test_execution::TestStatus) -> &'static str {
    match status {
        test_execution::TestStatus::NotExecuted => "not_executed",
        test_execution::TestStatus::InProgress => "in_progress",
        test_execution::TestStatus::Completed => "completed",
        test_execution::TestStatus::Queued => "queued",
        test_execution::TestStatus::Cancelled => "cancelled",
        test_execution::TestStatus::Failed => "failed",
    }
}
