//! Shared helpers for relay HTTP handlers (auth envelopes and error mapping).

use super::*;

/// PURPOSE: Require a wired device repository or return 503.
/// USE: Legacy relay handlers before store calls.
pub(super) async fn legacy_repository(
    state: &Arc<AppState>,
    headers: &HeaderMap,
) -> Result<(HeaderMap, Arc<dyn DeviceRepository>), Box<axum::response::Response>> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_handlers::shared", "legacy_repository");
    // Auth: require valid bearer token with administrator role.
    let response_headers = auth::authorize_request(state, headers, Some("admin"))
        .await
        .map_err(|error| Box::new(error.into_response()))?;
    let repository = state.device_repository.clone().ok_or_else(|| {
        // Guard: device repository is not wired in this process.
        Box::new(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        ))
    })?;
    Ok((response_headers, repository))
}
