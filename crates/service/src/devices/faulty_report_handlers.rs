use std::{
    path::{Path as FilePath, PathBuf},
    sync::Arc,
};

use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{auth, error::ErrorResponse, events::ServerEvent, state::AppState};

use super::{
    CompatibilityErrorResponse, FaultyReportCreateRequest, FaultyReportDetail, FaultyReportRecord,
    FaultyReportStatusRequest, MessageResponse, SuccessResponse, error_response,
    repository::DeviceRepository, repository_error, repository_types::FaultyReportCreate,
};

const ALLOWED_EXTENSIONS: &[&str] = &["txt", "log", "pdf", "png", "jpg", "jpeg", "gif", "zip"];

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

fn report_id(value: &str) -> Result<Uuid, Box<axum::response::Response>> {
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

fn safe_file_name(value: &str) -> Option<String> {
    let name = FilePath::new(value).file_name()?.to_str()?;
    let extension = FilePath::new(name)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    ALLOWED_EXTENSIONS
        .contains(&extension.as_str())
        .then(|| name.split_whitespace().collect::<Vec<_>>().join("_"))
}

fn safe_segment(value: &str) -> bool {
    let mut components = FilePath::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

async fn remove_pending(path: Option<&PathBuf>) {
    if let Some(path) = path {
        let _ = tokio::fs::remove_file(path).await;
    }
}

#[utoipa::path(post, path = "/api/v1/device/faulty/report", tag = "Faulty Reports", summary = "Submit a faulty build report", description = "Streams one bounded optional attachment, safely copies requested execution logs, commits the pending report and alert, then emits the browser alert.", security(("bearer_auth" = [])), request_body(content = FaultyReportCreateRequest, content_type = "multipart/form-data"), responses((status = 201, description = "Faulty report created", body = FaultyReportRecord), (status = 400, description = "Invalid report or attachment", body = CompatibilityErrorResponse), (status = 401, description = "Authentication required or token subject missing", body = CompatibilityErrorResponse), (status = 404, description = "Release not found", body = MessageResponse), (status = 500, description = "Report persistence or file storage failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, None).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let Some(user_id) = auth::token_subject(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"message": "Unauthorized: userId missing"})),
        )
            .into_response();
    };
    if tokio::fs::create_dir_all(&state.config.device.faulty_report_upload_dir)
        .await
        .is_err()
    {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Failed to prepare upload directory",
        );
    }

    let mut device_type = None;
    let mut device_family = None;
    let mut release_id = None;
    let mut description = None;
    let mut test_execution_id = None;
    let mut attach_logs = false;
    let mut pending_file = None;
    let mut stored_name = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(_) => {
                remove_pending(pending_file.as_ref()).await;
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "validation",
                    "Invalid multipart upload",
                );
            }
        };
        match field.name() {
            Some("deviceType") => device_type = field.text().await.ok(),
            Some("deviceFamily") => device_family = field.text().await.ok(),
            Some("releaseId") => release_id = field.text().await.ok(),
            Some("description") => description = field.text().await.ok(),
            Some("testExecutionId") => test_execution_id = field.text().await.ok(),
            Some("attachLogs") => attach_logs = field.text().await.ok().as_deref() == Some("true"),
            Some("image") => {
                let Some(name) = field.file_name().and_then(safe_file_name) else {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "validation",
                        "File type not supported. Allowed: .txt, .log, .pdf, images, .zip",
                    );
                };
                let name = format!("{}_{}", chrono::Utc::now().timestamp_millis(), name);
                let path = FilePath::new(&state.config.device.faulty_report_upload_dir)
                    .join(format!(".pending-{}", Uuid::new_v4()));
                let mut output = match tokio::fs::File::create(&path).await {
                    Ok(output) => output,
                    Err(_) => {
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "internal",
                            "Failed to store attachment",
                        );
                    }
                };
                let mut total = 0_u64;
                let mut field = field;
                loop {
                    match field.chunk().await {
                        Ok(Some(chunk)) => {
                            total = total.saturating_add(chunk.len() as u64);
                            if total > state.config.device.faulty_report_max_bytes {
                                let _ = tokio::fs::remove_file(&path).await;
                                return error_response(
                                    StatusCode::BAD_REQUEST,
                                    "validation",
                                    "File size exceeds 10MB limit",
                                );
                            }
                            if output.write_all(&chunk).await.is_err() {
                                let _ = tokio::fs::remove_file(&path).await;
                                return error_response(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "internal",
                                    "Failed to store attachment",
                                );
                            }
                        }
                        Ok(None) => break,
                        Err(_) => {
                            let _ = tokio::fs::remove_file(&path).await;
                            return error_response(
                                StatusCode::BAD_REQUEST,
                                "validation",
                                "Invalid multipart upload",
                            );
                        }
                    }
                }
                pending_file = Some(path);
                stored_name = Some(name);
            }
            _ => {}
        }
    }

    let (Some(device_type), Some(device_family), Some(release_id), Some(description)) =
        (device_type, device_family, release_id, description)
    else {
        remove_pending(pending_file.as_ref()).await;
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"message": "deviceType, deviceFamily, releaseId and description are required"})),
        )
            .into_response();
    };
    let id = Uuid::new_v4();
    let creation = match repository
        .create_faulty_report(&FaultyReportCreate {
            id,
            release_id: release_id.clone(),
            device_type,
            device_family,
            description,
            created_by: user_id,
            test_execution_id,
        })
        .await
    {
        Ok(Some(creation)) => creation,
        Ok(None) => {
            remove_pending(pending_file.as_ref()).await;
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"message": format!("Release {release_id} not found")})),
            )
                .into_response();
        }
        Err(error) => {
            remove_pending(pending_file.as_ref()).await;
            return repository_error(error, "creating faulty report");
        }
    };

    let report_dir =
        FilePath::new(&state.config.device.faulty_report_upload_dir).join(id.to_string());
    if tokio::fs::create_dir_all(&report_dir).await.is_err() {
        remove_pending(pending_file.as_ref()).await;
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Failed to store faulty report files",
        );
    }
    let file_path = match (pending_file, stored_name) {
        (Some(source), Some(name)) => {
            let target = report_dir.join(name);
            if tokio::fs::rename(source, &target).await.is_err() {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "Failed to store faulty report attachment",
                );
            }
            Some(target)
        }
        _ => None,
    };
    let logs_path = if attach_logs {
        match creation
            .last_test_execution_id
            .as_deref()
            .filter(|id| safe_segment(id))
        {
            Some(test_id) => {
                let source = FilePath::new(&state.config.tests.test_logs_dir)
                    .join(format!("{test_id}.json"));
                let target = report_dir.join(format!("test-logs-{test_id}.json"));
                match tokio::fs::copy(source, &target).await {
                    Ok(_) => Some(target),
                    Err(error) => {
                        tracing::warn!(%error, "failed to attach faulty report logs");
                        None
                    }
                }
            }
            None => None,
        }
    } else {
        None
    };
    match repository
        .finalize_faulty_report(
            id,
            file_path.as_deref().and_then(FilePath::to_str),
            logs_path.as_deref().and_then(FilePath::to_str),
        )
        .await
    {
        Ok(result) => {
            if let Err(error) = state
                .event_publisher
                .publish(ServerEvent::alert("device-alert", result.alert))
            {
                tracing::warn!(%error, report_id = %id, "failed to publish faulty report alert");
            }
            (StatusCode::CREATED, response_headers, Json(result.report)).into_response()
        }
        Err(error) => repository_error(error, "creating faulty report"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/faulty/report", tag = "Faulty Reports", summary = "List faulty reports", description = "Returns all retained faulty reports in descending creation order for administrator review.", security(("bearer_auth" = [])), responses((status = 200, description = "Faulty reports", body = [FaultyReportRecord]), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Report query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.faulty_reports().await {
        Ok(reports) => (response_headers, Json(reports)).into_response(),
        Err(error) => repository_error(error, "listing faulty reports"),
    }
}

async fn report_detail(
    state: &Arc<AppState>,
    repository: &Arc<dyn DeviceRepository>,
    id: Uuid,
) -> Result<serde_json::Value, Box<axum::response::Response>> {
    let Some(mut detail) = repository
        .faulty_report_by_id(id)
        .await
        .map_err(|error| repository_error(error, "getting faulty report"))?
    else {
        return Err(Box::new(
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"message": format!("Report {id} not found")})),
            )
                .into_response(),
        ));
    };
    let created_by = detail["report"]["createdBy"].as_str().unwrap_or_default();
    let Some(provider) = &state.identity_provider else {
        return Err(Box::new(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "auth_unavailable",
            "Authentication is disabled",
        )));
    };
    let user = provider.user_by_id(created_by).await.map_err(|error| {
        Box::new(error_response(
            error.status,
            "identity_error",
            &error.message,
        ))
    })?;
    detail["user"] = serde_json::json!({
        "id": user.id,
        "userName": user.username,
        "firstName": user.first_name,
        "lastName": user.last_name,
    });
    Ok(detail)
}

#[utoipa::path(get, path = "/api/v1/device/faulty/report/{id}", tag = "Faulty Reports", summary = "Get faulty report details", description = "Returns the report and build version enriched with the current Keycloak reporter projection.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Faulty report details", body = FaultyReportDetail), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report or identity user not found", body = CompatibilityErrorResponse), (status = 500, description = "Report or identity query failed", body = ErrorResponse), (status = 503, description = "Device persistence or identity provider unavailable", body = ErrorResponse)))]
pub(crate) async fn by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let id = match report_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    match report_detail(&state, &repository, id).await {
        Ok(detail) => (response_headers, Json(detail)).into_response(),
        Err(response) => *response,
    }
}

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
        Err(_) => return (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"message": format!("{missing} file not found for report {id}")}),
            ),
        )
            .into_response(),
    };
    let path = match tokio::fs::canonicalize(path).await {
        Ok(path) if path.starts_with(&root) => path,
        _ => return (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"message": format!("{missing} file not found for report {id}")}),
            ),
        )
            .into_response(),
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => return (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"message": format!("{missing} file not found for report {id}")}),
            ),
        )
            .into_response(),
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

#[utoipa::path(patch, path = "/api/v1/device/faulty/report/{id}/status", tag = "Faulty Reports", summary = "Approve or reject a faulty report", description = "Accepts only approved or rejected and atomically updates the report; approval also marks the associated release faulty.", params(("id" = String, Path)), request_body = FaultyReportStatusRequest, security(("bearer_auth" = [])), responses((status = 200, description = "Updated faulty report", body = FaultyReportRecord), (status = 400, description = "Invalid status", body = MessageResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report not found", body = MessageResponse), (status = 500, description = "Report update failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn update_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<FaultyReportStatusRequest>,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    if !matches!(request.status.as_str(), "approved" | "rejected") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"message": "Invalid status"})),
        )
            .into_response();
    }
    let id = match report_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    match repository
        .update_faulty_report_status(id, &request.status)
        .await
    {
        Ok(Some(report)) => (response_headers, Json(report)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"message": format!("Report {id} not found")})),
        )
            .into_response(),
        Err(error) => repository_error(error, "updating faulty report status"),
    }
}

#[utoipa::path(delete, path = "/api/v1/device/faulty/report/{id}", tag = "Faulty Reports", summary = "Delete a faulty report", description = "Hard-deletes the retained report row for compatibility; stored attachment and log files are intentionally left unchanged.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Faulty report deleted", body = SuccessResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report not found", body = MessageResponse), (status = 500, description = "Report deletion failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let id = match report_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    match repository.delete_faulty_report(id).await {
        Ok(true) => (response_headers, Json(serde_json::json!({"success": true}))).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"message": format!("Report {id} not found")})),
        )
            .into_response(),
        Err(error) => repository_error(error, "deleting faulty report"),
    }
}
