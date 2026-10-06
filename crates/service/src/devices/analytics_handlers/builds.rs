use super::*;

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
