use super::*;

const ALLOWED_EXTENSIONS: &[&str] = &["txt", "log", "pdf", "png", "jpg", "jpeg", "gif", "zip"];

pub(super) async fn context(
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

pub(super) fn report_id(value: &str) -> Result<Uuid, Box<axum::response::Response>> {
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

pub(super) fn safe_file_name(value: &str) -> Option<String> {
    let name = FilePath::new(value).file_name()?.to_str()?;
    let extension = FilePath::new(name)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    ALLOWED_EXTENSIONS
        .contains(&extension.as_str())
        .then(|| name.split_whitespace().collect::<Vec<_>>().join("_"))
}

pub(super) fn safe_segment(value: &str) -> bool {
    let mut components = FilePath::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

pub(super) async fn remove_pending(path: Option<&PathBuf>) {
    if let Some(path) = path {
        let _ = tokio::fs::remove_file(path).await;
    }
}
