use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use chrono::{SecondsFormat, Utc};

use crate::{auth, error::ErrorResponse, state::AppState};

use super::{DeviceCsvQuery, csv_export, error_response, repository_error};

#[utoipa::path(
    get,
    path = "/api/v1/device/export",
    tag = "Devices",
    summary = "Export devices as CSV",
    description = "Exports non-deleted devices using legacy filters, dynamic interface columns, legacy date formatting, and standards-compliant CSV quoting.",
    params(DeviceCsvQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Device CSV download", body = String, content_type = "text/csv", headers(("Content-Disposition" = String, description = "Timestamped CSV attachment filename"))),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 403, description = "Administrator role required", body = ErrorResponse),
        (status = 500, description = "CSV export failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn export_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DeviceCsvQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let devices = match repository.export_devices(&query).await {
        Ok(devices) => devices,
        Err(error) => return repository_error(error, "Failed to export devices"),
    };
    let csv = match csv_export::format_devices(&devices) {
        Ok(csv) => csv,
        Err(error) => {
            tracing::error!(%error, "device CSV formatting failed");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Failed to export devices",
            );
        }
    };
    let filename = format!(
        "attachment; filename=\"devices-{}.csv\"",
        Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    );
    let mut response = (response_headers, csv).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv"));
    if let Ok(value) = HeaderValue::from_str(&filename) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}
