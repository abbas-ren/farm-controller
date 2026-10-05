use super::{error_response, repository_error, test_export};
use crate::{auth, error::ErrorResponse, state::AppState};
use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use chrono::{SecondsFormat, Utc};
use std::sync::Arc;

#[utoipa::path(get, path = "/api/v1/device/test/export", tag = "Tests", summary = "Export test executions as CSV", description = "Exports current-user executions with legacy status, duration, count, percentage, and timestamp calculations.", security(("bearer_auth" = [])), responses((status = 200, description = "Test execution CSV", body = String, content_type = "text/csv", headers(("Content-Disposition" = String, description = "Timestamped CSV attachment filename"))), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "CSV export failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn export_tests(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let user_id = auth::token_subject(&headers).unwrap_or_else(|| "system".to_owned());
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let rows = match repository.test_export_rows(&user_id).await {
        Ok(rows) => rows,
        Err(error) => return repository_error(error, "Failed to export test executions"),
    };
    let csv = match test_export::format_rows(&rows, Utc::now()) {
        Ok(csv) => csv,
        Err(error) => return repository_error(error, "Failed to export test executions"),
    };
    let mut response = (response_headers, csv).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv"));
    let filename = format!(
        "attachment; filename=\"test-executions-{}.csv\"",
        Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    );
    if let Ok(value) = HeaderValue::from_str(&filename) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}
