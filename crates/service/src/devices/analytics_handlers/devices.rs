use super::*;

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
