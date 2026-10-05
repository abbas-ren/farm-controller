use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use chrono::{Datelike, Days, Duration, Months, NaiveDate, Utc};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::{auth, error::ErrorResponse, state::AppState};

use super::{
    DeviceStateAnalytics, DeviceStateDetailedAnalytics, error_response,
    repository::DeviceRepository, repository_error,
};

struct AnalyticsContext {
    response_headers: HeaderMap,
    repository: Arc<dyn DeviceRepository>,
    user_id: String,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct RecentExecutionQuery {
    count: Option<i64>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ExecutionDailyQuery {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceUsageQuery {
    day: Option<String>,
    device_id: Option<String>,
    device_family: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct TestAnalyticsQuery {
    #[serde(rename = "type")]
    analytics_type: String,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct TestDailyQuery {
    days: Option<i64>,
}

async fn analytics_context(
    state: &Arc<AppState>,
    headers: &HeaderMap,
) -> Result<AnalyticsContext, Box<axum::response::Response>> {
    let response_headers = auth::authorize_request(state, headers, None)
        .await
        .map_err(|error| Box::new(error.into_response()))?;
    let repository = state.device_repository.clone().ok_or_else(|| {
        Box::new(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        ))
    })?;
    Ok(AnalyticsContext {
        response_headers,
        repository,
        user_id: auth::token_subject(headers).unwrap_or_else(|| "system".to_owned()),
    })
}

#[utoipa::path(get, path = "/api/v1/device/analytics/state", tag = "Devices", summary = "Count approved devices by state", description = "Counts non-deleted approved devices by their current state; this inventory aggregate is intentionally not user-scoped.", security(("bearer_auth" = [])), responses((status = 200, description = "Approved device state counts", body = DeviceStateAnalytics), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn device_state(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match context.repository.device_state_analytics().await {
        Ok(analytics) => (context.response_headers, Json(analytics)).into_response(),
        Err(error) => repository_error(error, "Failed to get device state count"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/state/detailed", tag = "Devices", summary = "Group approved devices by family and state", description = "Groups non-deleted approved inventory by device family and current state for dashboard charts.", security(("bearer_auth" = [])), responses((status = 200, description = "Detailed approved device states", body = DeviceStateDetailedAnalytics), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn detailed_device_state(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match context.repository.detailed_device_state_analytics().await {
        Ok(analytics) => (context.response_headers, Json(analytics)).into_response(),
        Err(error) => repository_error(error, "Failed to get detailed device state analytics"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/execution", tag = "Devices", summary = "List recent user test executions", description = "Returns the current user's most recent execution activity, bounded by the requested count.", params(RecentExecutionQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Recent test execution activity"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn recent_executions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RecentExecutionQuery>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match context
        .repository
        .recent_execution_analytics(&context.user_id, query.count.unwrap_or(3))
        .await
    {
        Ok(executions) => (context.response_headers, Json(executions)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch recent test executions"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/execution/daily", tag = "Devices", summary = "Summarize user executions by day", description = "Aggregates the current user's execution statuses over an inclusive UTC date range; defaults to the trailing seven days.", params(ExecutionDailyQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Daily execution status summary"), (status = 400, description = "Invalid date", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn daily_executions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ExecutionDailyQuery>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let today = Utc::now().date_naive();
    let from = match query.from {
        Some(value) => match NaiveDate::parse_from_str(&value, "%Y-%m-%d") {
            Ok(value) => value,
            Err(_) => return invalid_date_response(),
        },
        None => today.checked_sub_days(Days::new(6)).unwrap_or(today),
    };
    let to = match query.to {
        Some(value) => match NaiveDate::parse_from_str(&value, "%Y-%m-%d") {
            Ok(value) => value,
            Err(_) => return invalid_date_response(),
        },
        None => today,
    };
    match context
        .repository
        .daily_execution_analytics(&context.user_id, from, to)
        .await
    {
        Ok(summary) => (context.response_headers, Json(summary)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch execution status daily summary"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/execution/{testId}", tag = "Devices", summary = "Get execution activity by test id", description = "Returns the historical execution activity projection for one test ID; legacy behavior is not owner-scoped.", params(("testId" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Test execution activity"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Test execution not found"), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn execution_by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match context.repository.execution_analytics_by_id(&test_id).await {
        Ok(Some(execution)) => (context.response_headers, Json(execution)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"message": "Test execution not found"})),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to fetch test execution by id"),
    }
}

fn invalid_date_response() -> axum::response::Response {
    error_response(
        StatusCode::BAD_REQUEST,
        "validation",
        "Dates must use YYYY-MM-DD format",
    )
}

#[utoipa::path(get, path = "/api/v1/device/analytics/builds/comparison", tag = "Devices", summary = "Compare recent completed builds for the current user", description = "Selects recent builds associated with the current user and compares system-wide completed execution results for those builds.", params(RecentExecutionQuery), security(("bearer_auth" = [])), responses((status = 200, description = "User build comparisons"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn build_comparisons(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RecentExecutionQuery>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let count = query.count.unwrap_or(3).clamp(1, 50);
    match context
        .repository
        .build_comparison_analytics(&context.user_id, count)
        .await
    {
        Ok(builds) => (context.response_headers, Json(builds)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch recent builds comparison"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/builds/comparison/{buildId}", tag = "Devices", summary = "Get a user build comparison", description = "Returns one comparison only when the build is associated with the current user; execution totals retain historical system-wide scope.", params(("buildId" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "User build comparison"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Build comparison not found"), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn build_comparison_by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match context
        .repository
        .build_comparison_analytics_by_id(&context.user_id, &build_id)
        .await
    {
        Ok(Some(build)) => (context.response_headers, Json(build)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"message": "Build comparison not found"})),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to fetch build comparison by id"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/builds/performance", tag = "Devices", summary = "Summarize recent build performance", description = "Returns historically unscoped pass/fail performance for the most recent builds, bounded by count.", params(RecentExecutionQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Build performance analytics"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn builds_performance(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RecentExecutionQuery>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let count = query.count.unwrap_or(10).clamp(1, 50);
    match context.repository.build_performance_analytics(count).await {
        Ok(builds) => (context.response_headers, Json(builds)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch builds performance analytics"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/builds/performance/{buildId}", tag = "Devices", summary = "Summarize one build's performance", description = "Returns the historically unscoped pass/fail performance projection for one build.", params(("buildId" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Build performance analytics"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn build_performance_by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match context
        .repository
        .build_performance_analytics_by_id(&build_id)
        .await
    {
        Ok(build) => (context.response_headers, Json(build)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch build performance analytics"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/usage/daily", tag = "Devices", summary = "Summarize device state durations", description = "Calculates approved-device state durations for one UTC day or the trailing 24 hours, with optional device and family filters.", params(DeviceUsageQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Device state duration analytics"), (status = 400, description = "Invalid day", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn daily_device_usage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DeviceUsageQuery>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let (start, end) = if let Some(day) = query.day.as_deref() {
        let day = match NaiveDate::parse_from_str(day, "%Y-%m-%d") {
            Ok(day) => day,
            Err(_) => return invalid_date_response(),
        };
        day_bounds(day)
    } else {
        let end = Utc::now();
        (end - Duration::hours(24), end)
    };
    match context
        .repository
        .device_usage_analytics(
            start,
            end,
            query.device_id.as_deref(),
            query.device_family.as_deref(),
        )
        .await
    {
        Ok(usage) => (context.response_headers, Json(usage)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch device usage analytics"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/usage/summary", tag = "Devices", summary = "Summarize device usage by week and month", description = "Returns four UTC weekly periods and four UTC monthly periods of approved-device usage.", security(("bearer_auth" = [])), responses((status = 200, description = "Weekly and monthly device usage analytics"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn device_usage_summary(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let today = Utc::now().date_naive();
    let current_week = today
        .checked_sub_days(Days::new(u64::from(today.weekday().num_days_from_sunday())))
        .unwrap_or(today);
    let current_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let mut weekly = Vec::with_capacity(4);
    let mut monthly = Vec::with_capacity(4);
    for offset in (0..4).rev() {
        let week_start = current_week
            .checked_sub_days(Days::new(offset * 7))
            .unwrap_or(current_week);
        let week_end = week_start
            .checked_add_days(Days::new(6))
            .unwrap_or(week_start);
        let (start, end) = day_range(week_start, week_end);
        let usage = match context
            .repository
            .device_usage_analytics(start, end, None, None)
            .await
        {
            Ok(usage) => usage,
            Err(error) => return repository_error(error, "Failed to fetch device usage summary"),
        };
        weekly.push(with_period(
            usage,
            "weekStart",
            week_start,
            "weekEnd",
            week_end,
        ));

        let month_start = current_month
            .checked_sub_months(Months::new(offset as u32))
            .unwrap_or(current_month);
        let month_end = month_start
            .checked_add_months(Months::new(1))
            .and_then(|next| next.checked_sub_days(Days::new(1)))
            .unwrap_or(month_start);
        let (start, end) = day_range(month_start, month_end);
        let usage = match context
            .repository
            .device_usage_analytics(start, end, None, None)
            .await
        {
            Ok(usage) => usage,
            Err(error) => return repository_error(error, "Failed to fetch device usage summary"),
        };
        monthly.push(with_period(
            usage,
            "monthStart",
            month_start,
            "monthEnd",
            month_end,
        ));
    }
    (
        context.response_headers,
        Json(serde_json::json!({"weekly": weekly, "monthly": monthly})),
    )
        .into_response()
}

fn day_bounds(day: NaiveDate) -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    day_range(day, day)
}

fn day_range(start: NaiveDate, end: NaiveDate) -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    let start = start.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
    let end = end
        .checked_add_days(Days::new(1))
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .unwrap_or_default()
        .and_utc()
        - Duration::nanoseconds(1);
    (start, end)
}

fn with_period(
    mut usage: serde_json::Value,
    start_key: &str,
    start: NaiveDate,
    end_key: &str,
    end: NaiveDate,
) -> serde_json::Value {
    if let Some(object) = usage.as_object_mut() {
        object.insert(start_key.to_owned(), serde_json::json!(start.to_string()));
        object.insert(end_key.to_owned(), serde_json::json!(end.to_string()));
    }
    usage
}

#[utoipa::path(get, path = "/api/v1/device/analytics/test", tag = "Devices", summary = "Get test execution analytics", description = "Returns either current-user execution aggregates or current-user in-progress analytics according to the required type selector.", params(TestAnalyticsQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Execution aggregate or in-progress analytics"), (status = 400, description = "Invalid analytics type", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn test_analytics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TestAnalyticsQuery>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match query.analytics_type.as_str() {
        "execution" => match context
            .repository
            .test_execution_analytics(&context.user_id)
            .await
        {
            Ok(analytics) => (context.response_headers, Json(analytics)).into_response(),
            Err(error) => repository_error(error, "Failed to get test execution analytics"),
        },
        "inProgress" => match context
            .repository
            .in_progress_test_analytics(&context.user_id)
            .await
        {
            Ok(executions) => (context.response_headers, Json(executions)).into_response(),
            Err(error) => repository_error(error, "Failed to get in-progress test analytics"),
        },
        _ => error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "Invalid type parameter. Use execution or inProgress",
        ),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/test/plan", tag = "Devices", summary = "Summarize test plans for the current user", description = "Groups the current user's test executions by plan and status for dashboard charts.", security(("bearer_auth" = [])), responses((status = 200, description = "Test plan status summary"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn test_plan_summary(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match context
        .repository
        .test_plan_analytics(&context.user_id)
        .await
    {
        Ok(summary) => (context.response_headers, Json(summary)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch test plan analytics"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/test/daily", tag = "Devices", summary = "Summarize recent test execution days", description = "Returns current-user execution status totals for a bounded number of recent UTC days.", params(TestDailyQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Daily test execution summary"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn daily_test_summary(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TestDailyQuery>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let days = query.days.unwrap_or(8).clamp(1, 90);
    match context
        .repository
        .daily_test_analytics(&context.user_id, days)
        .await
    {
        Ok(summary) => (context.response_headers, Json(summary)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch test execution daily summary"),
    }
}
