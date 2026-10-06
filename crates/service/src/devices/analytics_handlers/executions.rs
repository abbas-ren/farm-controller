use super::*;

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
