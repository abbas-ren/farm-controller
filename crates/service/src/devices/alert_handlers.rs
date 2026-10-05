use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use chrono::{DateTime, NaiveDate, Utc};

use crate::{auth, error::ErrorResponse, state::AppState};

use super::{
    AdminAlertListQuery, AlertList, AllAlertListQuery, MessageResponse, error_response,
    repository::DeviceRepository, repository_error,
};

async fn context(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    role: Option<&str>,
) -> Result<(HeaderMap, Arc<dyn DeviceRepository>), Box<axum::response::Response>> {
    let response_headers = auth::authorize_request(state, headers, role)
        .await
        .map_err(|error| Box::new(error.into_response()))?;
    let repository = state.device_repository.clone().ok_or_else(|| {
        Box::new(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        ))
    })?;
    Ok((response_headers, repository))
}

fn positive_page(value: Option<&str>, default: i64) -> i64 {
    value
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn alert_date(value: Option<&str>) -> Option<DateTime<Utc>> {
    let value = value?;
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        return Some(value.with_timezone(&Utc));
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)
        .map(|value| value.and_utc())
}

#[utoipa::path(get, path = "/api/v1/device/notification/alerts", tag = "Notifications", summary = "List active and recent administrative alerts", description = "Lists visible active/recent administrative alerts with legacy sorting and pagination; an empty result preserves the historical 500 response.", params(AdminAlertListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated alerts", body = AlertList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Alert query failed or returned no active alerts", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn admin_alerts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AdminAlertListQuery>,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let page = positive_page(query.page.as_deref(), 1);
    let limit = positive_page(query.limit.as_deref(), 10).min(100);
    match repository
        .admin_alerts(
            page,
            limit,
            query.sort_by.as_deref().unwrap_or("createdAt"),
            query.desc.as_deref().unwrap_or("true") == "true",
        )
        .await
    {
        Ok(alerts) if alerts.data.is_empty() => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "alert_error",
            "Failed to fetch alerts",
        ),
        Ok(alerts) => (response_headers, Json(alerts)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch alerts"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/notification/alerts/all", tag = "Notifications", summary = "List all alerts", description = "Lists all visible alerts with unread totals and tolerant optional date filters; invalid dates are ignored for compatibility.", params(AllAlertListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated alerts with unread count", body = AlertList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Alert query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn all_alerts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AllAlertListQuery>,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository
        .all_alerts(
            positive_page(query.page.as_deref(), 1),
            positive_page(query.limit.as_deref(), 20).min(100),
            alert_date(query.from.as_deref()),
            alert_date(query.to.as_deref()),
        )
        .await
    {
        Ok(alerts) => (response_headers, Json(alerts)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch alerts"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/notification/alerts/{id}/read", tag = "Notifications", summary = "Mark one alert as read", description = "Canonical authenticated mutation; unlike the retained administrator route, a missing alert returns 404.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Alert marked read", body = MessageResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Alert not found", body = MessageResponse), (status = 500, description = "Alert update failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn mark_single_read(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(alert_id): Path<String>,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, None).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.mark_alert_read(&alert_id).await {
        Ok(true) => (
            response_headers,
            Json(serde_json::json!({"message": "Alert marked as read"})),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"message": format!("Alert {alert_id} not found")})),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to mark alert as read"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/notification/alerts/read/{id}", tag = "Notifications", summary = "Administratively mark one alert as read", description = "Retained administrator mutation that acknowledges success even when no alert row matches the supplied ID.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Alert marked read", body = MessageResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Alert update failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn mark_read(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(alert_id): Path<String>,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.mark_alert_read(&alert_id).await {
        Ok(_) => (
            response_headers,
            Json(serde_json::json!({"message": "Marked read successful"})),
        )
            .into_response(),
        Err(error) => repository_error(error, "Alerts not found"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/notification/alerts/all/read", tag = "Notifications", summary = "Mark all alerts as read", description = "Preserves the legacy best-effort contract: persistence failures are logged and the route still acknowledges the request.", security(("bearer_auth" = [])), responses((status = 200, description = "Alerts marked read", body = MessageResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn mark_all_read(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    if let Err(error) = repository.mark_all_alerts_read().await {
        tracing::error!(error = %error, "failed to mark all alerts as read");
    }
    (
        response_headers,
        Json(serde_json::json!({"message": "Marked all as read successful"})),
    )
        .into_response()
}
