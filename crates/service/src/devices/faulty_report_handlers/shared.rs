//! Shared auth, path, and error helpers for faulty-report handlers.

use super::*;

const ALLOWED_EXTENSIONS: &[&str] = &["txt", "log", "pdf", "png", "jpg", "jpeg", "gif", "zip"];

/// PURPOSE: Authorize admin and resolve repository for faulty-report handlers.
/// USE: All faulty-report HTTP entry points.
pub(super) async fn context(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    role: Option<&str>,
) -> Result<(HeaderMap, Arc<dyn DeviceRepository>), Box<axum::response::Response>> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_handlers::shared", "context");
    let response_headers = auth::authorize_request(state, headers, role)
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

/// PURPOSE: Parse and validate the faulty-report path id.
/// USE: By-id faulty-report handlers.
pub(super) fn report_id(value: &str) -> Result<Uuid, Box<axum::response::Response>> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_handlers::shared", "report_id");
    Uuid::parse_str(value).map_err(|_| {
        Box::new(
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"message": format!("Report {value} not found")})),
            )
                .into_response(),
        )
    })
}

/// PURPOSE: Sanitize uploaded file names to a path-safe basename.
/// USE: Faulty-report create/upload.
pub(super) fn safe_file_name(value: &str) -> Option<String> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_handlers::shared", "safe_file_name");
    let name = FilePath::new(value).file_name()?.to_str()?;
    let extension = FilePath::new(name)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    ALLOWED_EXTENSIONS
        .contains(&extension.as_str())
        .then(|| name.split_whitespace().collect::<Vec<_>>().join("_"))
}

/// PURPOSE: Reject path traversal segments in faulty-report paths.
/// USE: Download path construction.
pub(super) fn safe_segment(value: &str) -> bool {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_handlers::shared", "safe_segment");
    let mut components = FilePath::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

/// PURPOSE: Delete a pending upload staging file on failure.
/// USE: Create/upload error paths.
pub(super) async fn remove_pending(path: Option<&PathBuf>) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_handlers::shared", "remove_pending");
    if let Some(path) = path {
        let _ = tokio::fs::remove_file(path).await;
    }
}
