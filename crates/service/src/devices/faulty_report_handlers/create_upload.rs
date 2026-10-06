use super::*;

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
