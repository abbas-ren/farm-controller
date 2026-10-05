use super::{
    BuildFilters, BuildFlagRequest, BuildFlagResponse, BuildList, BuildListQuery,
    BuildUploadInitRequest, BuildUploadInitResponse, BuildUploadResponse, error_response,
    repository_error, upload_events,
};
use crate::{auth, error::ErrorResponse, events::ServerEvent, state::AppState};
use axum::{
    Json,
    extract::{Multipart, Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

#[utoipa::path(put, path = "/api/v1/device/build/{id}/flag", tag = "Builds", summary = "Flag or unflag a build", description = "Atomically updates the release fault/status fields, then publishes the committed build event and persisted alert.", params(("id" = String, Path)), security(("bearer_auth" = [])), request_body = BuildFlagRequest, responses((status = 200, description = "Build flag updated", body = BuildFlagResponse), (status = 400, description = "isFaulty boolean is required", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Build not found or update failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn flag_build(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
    payload: Result<Json<BuildFlagRequest>, JsonRejection>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "validation",
                "isFaulty boolean is required",
            );
        }
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.flag_build(&build_id, &request).await {
        Ok(Some(result)) => {
            publish_build_event(
                &state,
                "build_flagged",
                serde_json::json!({
                    "releaseId": build_id,
                    "isFaulty": request.is_faulty,
                    "version": result.version,
                    "deviceType": result.device_type,
                }),
            );
            publish_alert(&state, result.alert);
            (
                response_headers,
                Json(serde_json::json!({"id": build_id, "isFaulty": request.is_faulty})),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &format!("Release not found: {build_id}"),
        ),
        Err(error) => repository_error(error, "Failed to flag build"),
    }
}

#[utoipa::path(delete, path = "/api/v1/device/build/{id}", tag = "Builds", summary = "Delete a build", description = "Deletes the release, then best-effort removes safe local and controller IPL artifacts before publishing the committed alert/event.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 204, description = "Build deleted"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Build not found or deletion failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn delete_build(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
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
    match repository.delete_build(&build_id).await {
        Ok(Some(target)) => {
            if safe_segment(&target.folder_name) && safe_segment(&target.version) {
                let path = std::path::Path::new(&state.config.device.build_artifacts_dir)
                    .join(&target.folder_name)
                    .join(&target.version);
                if let Err(error) = tokio::fs::remove_dir_all(path).await
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(%error, family = %target.device_family, "failed to delete build artifacts");
                }
            }
            if let Err(error) =
                crate::workers::remove_build_ipl(&state, &target.device_family, &target.version)
                    .await
            {
                tracing::error!(%error, family = %target.device_family, version = %target.version, "failed to remove build IPL payload");
            }
            publish_build_event(
                &state,
                "build_deleted",
                serde_json::json!({
                    "releaseId": build_id,
                    "version": target.version,
                    "deviceType": target.device_type,
                }),
            );
            publish_alert(&state, target.alert);
            (response_headers, StatusCode::NO_CONTENT).into_response()
        }
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &format!("Release not found: {build_id}"),
        ),
        Err(error) => repository_error(error, "Failed to delete build"),
    }
}

fn publish_build_event(state: &AppState, event: &str, payload: serde_json::Value) {
    if let Err(error) = state.event_publisher.publish(ServerEvent {
        event: event.to_owned(),
        payload,
        room: None,
    }) {
        tracing::warn!(%error, event, "failed to publish build event");
    }
}

fn publish_alert(state: &AppState, alert: serde_json::Value) {
    if let Err(error) = state
        .event_publisher
        .publish(ServerEvent::alert("device-alert", alert))
    {
        tracing::warn!(%error, "failed to publish build alert");
    }
}

fn safe_segment(value: &str) -> bool {
    let mut components = std::path::Path::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
struct BuildUploadMultipart {
    #[schema(example = "8e8f632f-18ee-469f-a472-79245f37c842")]
    #[serde(rename = "uploadId")]
    upload_id: String,
    #[schema(format = Binary)]
    file: String,
    #[schema(example = "nightly")]
    tag: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/v1/device/build/upload/custom",
    tag = "Builds",
    summary = "Upload a custom build archive",
    description = "Streams a deviceType__version.zip archive, validates its firmware layout, commits the release, stages NFS/TFTP artifacts, and publishes upload events. A tag is required and becomes the custom version suffix.",
    security(("bearer_auth" = [])),
    request_body(content = BuildUploadMultipart, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "Custom build committed and staged", body = BuildUploadResponse),
        (status = 400, description = "Multipart fields, tag, ZIP name, size, or firmware layout are invalid", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 500, description = "Upload storage or ingestion failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn upload_custom(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> axum::response::Response {
    upload_multipart(state, headers, multipart, true).await
}

#[utoipa::path(
    post,
    path = "/api/v1/device/build/upload",
    tag = "Builds",
    summary = "Upload an official build archive",
    description = "Streams and validates a deviceType__version.zip archive, commits and stages the release, then queues its first device-type flash after commit.",
    security(("bearer_auth" = [])),
    request_body(content = BuildUploadMultipart, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "Official build committed, staged, and queued", body = BuildUploadResponse),
        (status = 400, description = "Multipart fields, ZIP name, size, or firmware layout are invalid", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 403, description = "Administrator role required", body = ErrorResponse),
        (status = 500, description = "Upload storage or ingestion failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn upload_official(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> axum::response::Response {
    upload_multipart(state, headers, multipart, false).await
}

async fn upload_multipart(
    state: Arc<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
    is_custom: bool,
) -> axum::response::Response {
    let required_role = (!is_custom).then_some("admin");
    let response_headers = match auth::authorize_request(&state, &headers, required_role).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let user_id = upload_events::user_id(&headers);
    let mut upload_id = None;
    let mut tag = None;
    let mut filename = None;
    let mut temporary_path = None;
    while let Ok(Some(mut field)) = multipart.next_field().await {
        match field.name() {
            Some("uploadId") => upload_id = field.text().await.ok(),
            Some("tag") => tag = field.text().await.ok(),
            Some("file") => {
                let original = field
                    .file_name()
                    .and_then(|name| std::path::Path::new(name).file_name())
                    .and_then(|name| name.to_str())
                    .map(str::to_owned);
                let Some(original) = original else {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "validation",
                        "No file uploaded",
                    );
                };
                if !original.to_lowercase().ends_with(".zip") {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "validation",
                        "Only .zip files are supported for build uploads",
                    );
                }
                if tokio::fs::create_dir_all(&state.config.device.build_upload_dir)
                    .await
                    .is_err()
                {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal",
                        "Failed to prepare upload directory",
                    );
                }
                let path = std::path::Path::new(&state.config.device.build_upload_dir)
                    .join(format!("multipart-{}", uuid::Uuid::new_v4()));
                let mut output = match tokio::fs::File::create(&path).await {
                    Ok(file) => file,
                    Err(_) => {
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "internal",
                            "Failed to create upload",
                        );
                    }
                };
                let mut total = 0_u64;
                loop {
                    match field.chunk().await {
                        Ok(Some(chunk)) => {
                            total = total.saturating_add(chunk.len() as u64);
                            if total > state.config.device.build_upload_max_bytes {
                                let _ = tokio::fs::remove_file(&path).await;
                                return error_response(
                                    StatusCode::BAD_REQUEST,
                                    "validation",
                                    "File too large",
                                );
                            }
                            if output.write_all(&chunk).await.is_err() {
                                let _ = tokio::fs::remove_file(&path).await;
                                return error_response(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "internal",
                                    "Failed to store upload",
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
                filename = Some(original);
                temporary_path = Some(path);
            }
            _ => {}
        }
    }
    let Some(path) = temporary_path else {
        return error_response(StatusCode::BAD_REQUEST, "validation", "No file uploaded");
    };
    let Some(filename) = filename else {
        let _ = tokio::fs::remove_file(path).await;
        return error_response(StatusCode::BAD_REQUEST, "validation", "No file uploaded");
    };
    let Some(upload_id) = upload_id.filter(|value| !value.is_empty()) else {
        let _ = tokio::fs::remove_file(path).await;
        return error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "uploadId is required",
        );
    };
    if is_custom && tag.as_deref().is_none_or(|tag| tag.trim().is_empty()) {
        let _ = tokio::fs::remove_file(path).await;
        return error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "tag is required for custom builds",
        );
    }
    let Some(repository) = &state.device_repository else {
        let _ = tokio::fs::remove_file(path).await;
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let request = crate::devices::repository_types::BuildUploadFinalization {
        source_path: path.clone(),
        artifacts_root: state.config.device.build_artifacts_dir.clone().into(),
        upload_id: upload_id.clone(),
        filename: filename.clone(),
        tag,
        is_custom,
        user_id: user_id.clone(),
        max_expanded_bytes: state.config.device.build_upload_max_bytes.saturating_mul(4),
    };
    if let Err(error) = repository
        .mark_build_upload_started(&upload_id, &user_id)
        .await
    {
        let _ = tokio::fs::remove_file(path).await;
        return repository_error(error, "Failed to start build upload");
    }
    upload_events::progress(&state, &user_id, &upload_id, 0);
    match repository.finalize_build_upload(&request).await {
        Ok(result) => {
            upload_events::progress(&state, &user_id, &upload_id, 100);
            upload_events::completed(&state, &request, result).await;
            (response_headers, Json(serde_json::json!({"message": "Upload successful", "uploadId": upload_id, "filename": filename}))).into_response()
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(path).await;
            let _ = repository.mark_build_upload_failed(&upload_id).await;
            upload_events::failure(&state, &user_id, &upload_id, &error.to_string());
            tracing::error!(%error, "multipart build upload failed");
            error_response(StatusCode::BAD_REQUEST, "upload_failed", &error.to_string())
        }
    }
}

#[utoipa::path(post, path = "/api/v1/device/build/upload/init", tag = "Builds", summary = "Initialize a build upload", description = "Creates the legacy durable upload session used to correlate one through five multipart or TUS archive uploads.", security(("bearer_auth" = [])), request_body = BuildUploadInitRequest, responses((status = 201, description = "Upload initialized", body = BuildUploadInitResponse), (status = 400, description = "Invalid file count", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Upload initialization failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn init_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<BuildUploadInitRequest>, JsonRejection>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(error) => {
            let message = error.body_text();
            return error_response(StatusCode::BAD_REQUEST, "validation", &message);
        }
    };
    if !(1..=5).contains(&request.file_count) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "fileCount is required and must be between 1 and 5",
        );
    }
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let user_id = upload_events::user_id(&headers);
    match repository
        .init_build_upload(request.file_count, &user_id)
        .await
    {
        Ok(upload_id) => (
            StatusCode::CREATED,
            response_headers,
            Json(BuildUploadInitResponse { upload_id }),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to initialize build upload"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/build/filters", tag = "Builds", summary = "List build filters", description = "Returns deterministic distinct device-family and device-type values from retained release rows.", security(("bearer_auth" = [])), responses((status = 200, description = "Available build filters", body = BuildFilters), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Build filter query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn build_filters(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
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
    match repository.build_filters().await {
        Ok(filters) => (response_headers, Json(filters)).into_response(),
        Err(error) => repository_error(error, "Failed to get build filters"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/build/{id}", tag = "Builds", summary = "Get a build", description = "Returns the complete heterogeneous legacy release row so existing frontend fields remain available.", params(("id" = String, Path, description = "Build ID")), security(("bearer_auth" = [])), responses((status = 200, description = "Complete legacy release row"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Build not found", body = ErrorResponse), (status = 500, description = "Build query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn build_by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
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
    match repository.build_by_id(&build_id).await {
        Ok(Some(build)) => (response_headers, Json(build)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Build not found"),
        Err(error) => repository_error(error, "Failed to get build"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/build", tag = "Builds", summary = "List builds", description = "Returns paginated complete legacy release rows with allowlisted sorting and family, type, version, fault, and search filters.", params(BuildListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated builds", body = BuildList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Build query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list_builds(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<BuildListQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
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
    match repository.list_builds(&query).await {
        Ok(builds) => (response_headers, Json(builds)).into_response(),
        Err(error) => repository_error(error, "Failed to list builds"),
    }
}
