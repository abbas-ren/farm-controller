use super::*;

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
