use super::*;

pub(super) struct AnalyticsContext {
    pub(super) response_headers: HeaderMap,
    pub(super) repository: Arc<dyn DeviceRepository>,
    pub(super) user_id: String,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct RecentExecutionQuery {
    pub(super) count: Option<i64>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ExecutionDailyQuery {
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceUsageQuery {
    pub(super) day: Option<String>,
    pub(super) device_id: Option<String>,
    pub(super) device_family: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct TestAnalyticsQuery {
    #[serde(rename = "type")]
    pub(super) analytics_type: String,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct TestDailyQuery {
    pub(super) days: Option<i64>,
}

pub(super) async fn analytics_context(
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

pub(super) fn invalid_date_response() -> axum::response::Response {
    error_response(
        StatusCode::BAD_REQUEST,
        "validation",
        "Dates must use YYYY-MM-DD format",
    )
}
