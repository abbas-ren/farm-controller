use super::*;

async fn download(
    state: Arc<AppState>,
    headers: HeaderMap,
    id: String,
    field: &str,
    missing: &str,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let id = match report_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    let detail = match repository.faulty_report_by_id(id).await {
        Ok(Some(detail)) => detail,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"message": format!("Report {id} not found")})),
            )
                .into_response();
        }
        Err(error) => return repository_error(error, "getting faulty report"),
    };
    let Some(path) = detail["report"][field].as_str() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"message": format!("No {missing} found for report {id}")})),
        )
            .into_response();
    };
    let root = match tokio::fs::canonicalize(&state.config.device.faulty_report_upload_dir).await {
        Ok(root) => root,
        Err(_) => return missing_file(missing, id),
    };
    let path = match tokio::fs::canonicalize(path).await {
        Ok(path) if path.starts_with(&root) => path,
        _ => return missing_file(missing, id),
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => return missing_file(missing, id),
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download.bin");
    let mut response = (response_headers, Body::from(bytes)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        content_type(name)
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream")),
    );
    if let Ok(value) = header::HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"",
        name.replace(['\r', '\n', '"'], "_")
    )) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

fn missing_file(missing: &str, id: Uuid) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"message": format!("{missing} file not found for report {id}")})),
    )
        .into_response()
}

fn content_type(name: &str) -> &'static str {
    match FilePath::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "txt" | "log" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

#[utoipa::path(get, path = "/api/v1/device/faulty/report/{id}/file", tag = "Faulty Reports", summary = "Download a faulty report attachment", description = "Serves the stored attachment only after canonical upload-root containment, with a sanitized attachment filename and detected media type.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Faulty report attachment; content type reflects the stored file", body = String, content_type = "application/octet-stream"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report or attachment not found", body = MessageResponse), (status = 500, description = "Report query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn download_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> axum::response::Response {
    download(state, headers, id, "filePath", "attachment").await
}

#[utoipa::path(get, path = "/api/v1/device/faulty/report/{id}/logs", tag = "Faulty Reports", summary = "Download attached test logs", description = "Serves the copied execution logs only after canonical upload-root containment and filename sanitization.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Attached test logs", body = String, content_type = "application/json"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report or logs not found", body = MessageResponse), (status = 500, description = "Report query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn download_logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> axum::response::Response {
    download(state, headers, id, "logsPath", "logs").await
}
