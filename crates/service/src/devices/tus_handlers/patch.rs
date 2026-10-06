use std::{path::Path, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use uuid::Uuid;

use crate::{error::ErrorResponse, state::AppState};

use super::{
    super::{error_response, repository_types::BuildUploadFinalization, tus_store, upload_events},
    *,
};

#[utoipa::path(
    patch,
    path = "/api/v1/device/build/upload/tus/{id}",
    tag = "Builds",
    summary = "Append bytes to a resumable build upload",
    description = "Appends an offset-checked chunk. The final chunk validates and commits the release, stages artifacts, and queues official builds.",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "TUS upload resource ID"),
        ("Tus-Resumable" = String, Header, description = "Must be 1.0.0"),
        ("Upload-Offset" = u64, Header, description = "Expected current byte offset")
    ),
    request_body(content = Vec<u8>, content_type = "application/offset+octet-stream"),
    responses(
        (status = 204, description = "Chunk accepted", headers(
            ("Tus-Resumable" = String, description = "Selected TUS protocol version; always 1.0.0"),
            ("Upload-Offset" = u64, description = "New persisted byte offset")
        )),
        (status = 400, description = "Upload offset or completion metadata is invalid", body = ErrorResponse),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 404, description = "Upload not found"),
        (status = 409, description = "Offset does not match"),
        (status = 412, description = "Tus-Resumable is missing or unsupported"),
        (status = 413, description = "Chunk exceeds declared upload length"),
        (status = 415, description = "Content-Type must be application/offset+octet-stream"),
        (status = 500, description = "Upload storage or finalization failed"),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn patch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<Uuid>,
    body: Bytes,
) -> axum::response::Response {
    let response_headers = match authorize(&state, &headers).await {
        Ok(headers) => headers,
        Err(response) => return *response,
    };
    if let Err(status) = validate_version(&headers) {
        return status.into_response();
    }
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        != Some("application/offset+octet-stream")
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let offset = match integer_header(&headers, "upload-offset") {
        Ok(offset) => offset,
        Err(status) => return status.into_response(),
    };
    match tus_store::append(
        Path::new(&state.config.device.build_upload_dir),
        id,
        offset,
        &body,
    )
    .await
    {
        Ok(upload) => {
            let user_id = upload_events::user_id(&headers);
            if let Some(upload_id) = upload.metadata.get("uploadId") {
                let percent = upload
                    .offset
                    .saturating_mul(100)
                    .checked_div(upload.length)
                    .unwrap_or(100);
                upload_events::progress(&state, &user_id, upload_id, percent);
            }
            if upload.offset == upload.length {
                let Some(repository) = &state.device_repository else {
                    // Rewind the completed chunk so the client can retry after persistence recovers.
                    let _ = tus_store::rollback(
                        Path::new(&state.config.device.build_upload_dir),
                        id,
                        offset,
                    )
                    .await;
                    return error_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "device_unavailable",
                        "Device persistence is unavailable",
                    );
                };
                let Some(filename) = upload.metadata.get("filename").cloned() else {
                    let _ = tus_store::rollback(
                        Path::new(&state.config.device.build_upload_dir),
                        id,
                        offset,
                    )
                    .await;
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "validation",
                        "Missing required metadata: filename, uploadId",
                    );
                };
                let Some(upload_id) = upload.metadata.get("uploadId").cloned() else {
                    let _ = tus_store::rollback(
                        Path::new(&state.config.device.build_upload_dir),
                        id,
                        offset,
                    )
                    .await;
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "validation",
                        "Missing required metadata: filename, uploadId",
                    );
                };
                let request = BuildUploadFinalization {
                    source_path: tus_store::data_path(
                        Path::new(&state.config.device.build_upload_dir),
                        id,
                    ),
                    artifacts_root: state.config.device.build_artifacts_dir.clone().into(),
                    upload_id,
                    filename,
                    tag: upload.metadata.get("tag").cloned(),
                    is_custom: upload
                        .metadata
                        .get("tag")
                        .is_some_and(|tag| !tag.trim().is_empty()),
                    user_id: user_id.clone(),
                    max_expanded_bytes: state
                        .config
                        .device
                        .build_upload_max_bytes
                        .saturating_mul(4),
                };
                match repository.finalize_build_upload(&request).await {
                    Ok(result) => upload_events::completed(&state, &request, result).await,
                    Err(error) => {
                        // The durable offset must match the last committed file position on retry.
                        let _ = tus_store::rollback(
                            Path::new(&state.config.device.build_upload_dir),
                            id,
                            offset,
                        )
                        .await;
                        let _ = repository
                            .mark_build_upload_failed(&request.upload_id)
                            .await;
                        upload_events::failure(
                            &state,
                            &user_id,
                            &request.upload_id,
                            &error.to_string(),
                        );
                        tracing::error!(%error, upload_id = %id, "failed to finalize TUS build upload");
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "upload_finalization_failed",
                            "Failed to process completed build upload",
                        );
                    }
                }
            }
            let mut response_headers = protocol_headers(response_headers);
            insert_integer(&mut response_headers, "upload-offset", upload.offset);
            (StatusCode::NO_CONTENT, response_headers).into_response()
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            StatusCode::CONFLICT.into_response()
        }
        Err(error) if error.to_string().contains("offset mismatch") => {
            StatusCode::CONFLICT.into_response()
        }
        Err(error) if error.to_string().contains("exceeds upload length") => {
            StatusCode::PAYLOAD_TOO_LARGE.into_response()
        }
        Err(error) => {
            tracing::error!(%error, "failed to append TUS upload");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
