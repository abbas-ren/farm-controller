use super::*;

pub(super) async fn legacy_repository(
    state: &Arc<AppState>,
    headers: &HeaderMap,
) -> Result<(HeaderMap, Arc<dyn DeviceRepository>), Box<axum::response::Response>> {
    let response_headers = auth::authorize_request(state, headers, Some("admin"))
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
