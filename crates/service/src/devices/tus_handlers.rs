use std::{collections::BTreeMap, path::Path, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use uuid::Uuid;

use crate::{auth, error::ErrorResponse, state::AppState};

use super::{
    error_response, repository_error, repository_types::BuildUploadFinalization, tus_store,
    upload_events,
};

const TUS_VERSION: &str = "1.0.0";

#[utoipa::path(
    options,
    path = "/api/v1/device/build/upload/tus",
    tag = "Builds",
    summary = "Discover resumable upload capabilities",
    description = "Returns TUS 1.0 protocol, version, and creation extension headers.",
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "TUS capabilities returned in response headers", headers(
            ("Tus-Resumable" = String, description = "Selected TUS protocol version; always 1.0.0"),
            ("Tus-Version" = String, description = "Supported TUS versions; currently 1.0.0"),
            ("Tus-Extension" = String, description = "Supported extensions; currently creation"),
            ("Tus-Max-Size" = u64, description = "Maximum archive size in bytes")
        )),
        (status = 401, description = "Authentication required", body = ErrorResponse)
    )
)]
pub async fn options(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let response_headers = match authorize(&state, &headers).await {
        Ok(headers) => headers,
        Err(response) => return *response,
    };
    let mut response_headers = protocol_headers(response_headers);
    insert_integer(
        &mut response_headers,
        "tus-max-size",
        state.config.device.build_upload_max_bytes,
    );
    (StatusCode::NO_CONTENT, response_headers).into_response()
}

#[utoipa::path(
    options,
    path = "/api/v1/device/build/upload/tus/{id}",
    tag = "Builds",
    summary = "Discover resumable upload resource capabilities",
    description = "Returns the same TUS 1.0 creation capabilities and configured size limit as the collection endpoint.",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "TUS upload resource ID")),
    responses(
        (status = 204, description = "TUS capabilities returned in response headers", headers(
            ("Tus-Resumable" = String, description = "Selected TUS protocol version; always 1.0.0"),
            ("Tus-Version" = String, description = "Supported TUS versions; currently 1.0.0"),
            ("Tus-Extension" = String, description = "Supported extensions; currently creation"),
            ("Tus-Max-Size" = u64, description = "Maximum archive size in bytes")
        )),
        (status = 401, description = "Authentication required", body = ErrorResponse)
    )
)]
pub async fn options_resource(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(_id): AxumPath<Uuid>,
) -> axum::response::Response {
    options(State(state), headers).await
}

#[utoipa::path(
    post,
    path = "/api/v1/device/build/upload/tus",
    tag = "Builds",
    summary = "Create a resumable build upload",
    description = "Creates a TUS 1.0 upload resource. Upload-Metadata must include base64 filename and uploadId values; tag selects a custom build.",
    security(("bearer_auth" = [])),
    params(
        ("Tus-Resumable" = String, Header, description = "Must be 1.0.0"),
        ("Upload-Length" = u64, Header, description = "Total archive bytes"),
        ("Upload-Metadata" = String, Header, description = "Comma-separated base64 metadata")
    ),
    responses(
        (status = 201, description = "Upload resource created", headers(
            ("Location" = String, description = "Relative resource path /api/v1/device/build/upload/tus/{id}"),
            ("Tus-Resumable" = String, description = "Selected TUS protocol version; always 1.0.0")
        )),
        (status = 400, description = "TUS headers or metadata are invalid"),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 412, description = "Tus-Resumable is missing or unsupported"),
        (status = 413, description = "Upload exceeds the configured limit", body = ErrorResponse),
        (status = 500, description = "Upload storage or persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let response_headers = match authorize(&state, &headers).await {
        Ok(headers) => headers,
        Err(response) => return *response,
    };
    if let Err(status) = validate_version(&headers) {
        return status.into_response();
    }
    let length = match integer_header(&headers, "upload-length") {
        Ok(length) => length,
        Err(status) => return status.into_response(),
    };
    if length > state.config.device.build_upload_max_bytes {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "upload_too_large",
            "Upload exceeds configured maximum size",
        );
    }
    let metadata = match parse_metadata(&headers) {
        Ok(metadata) => metadata,
        Err(status) => return status.into_response(),
    };
    let user_id = upload_events::user_id(&headers);
    let root = Path::new(&state.config.device.build_upload_dir);
    match tus_store::create(root, length, metadata).await {
        Ok(upload) => {
            if let Some(upload_id) = upload.metadata.get("uploadId") {
                let Some(repository) = &state.device_repository else {
                    return error_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "device_unavailable",
                        "Device persistence is unavailable",
                    );
                };
                if let Err(error) = repository
                    .mark_build_upload_started(upload_id, &user_id)
                    .await
                {
                    return repository_error(error, "Failed to start build upload");
                }
                upload_events::progress(&state, &user_id, upload_id, 0);
            }
            let mut response_headers = protocol_headers(response_headers);
            let location = format!("/api/v1/device/build/upload/tus/{}", upload.id);
            if let Ok(value) = HeaderValue::from_str(&location) {
                response_headers.insert(header::LOCATION, value);
            }
            (StatusCode::CREATED, response_headers).into_response()
        }
        Err(error) => {
            tracing::error!(%error, "failed to create TUS upload");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Failed to create upload",
            )
        }
    }
}

#[utoipa::path(
    head,
    path = "/api/v1/device/build/upload/tus/{id}",
    tag = "Builds",
    summary = "Inspect a resumable build upload",
    description = "Returns the durable server offset and declared archive length so a TUS client can safely resume an interrupted upload.",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "TUS upload resource ID"),
        ("Tus-Resumable" = String, Header, description = "Must be 1.0.0")
    ),
    responses(
        (status = 200, description = "Current upload position", headers(
            ("Tus-Resumable" = String, description = "Selected TUS protocol version; always 1.0.0"),
            ("Upload-Offset" = u64, description = "Number of bytes stored"),
            ("Upload-Length" = u64, description = "Declared total archive bytes")
        )),
        (status = 401, description = "Authentication required", body = ErrorResponse),
        (status = 404, description = "Upload not found"),
        (status = 412, description = "Tus-Resumable is missing or unsupported"),
        (status = 500, description = "Upload storage inspection failed")
    )
)]
pub async fn head(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<Uuid>,
) -> axum::response::Response {
    let response_headers = match authorize(&state, &headers).await {
        Ok(headers) => headers,
        Err(response) => return *response,
    };
    if let Err(status) = validate_version(&headers) {
        return status.into_response();
    }
    match tus_store::load(Path::new(&state.config.device.build_upload_dir), id).await {
        Ok(upload) => {
            let mut response_headers = protocol_headers(response_headers);
            insert_integer(&mut response_headers, "upload-offset", upload.offset);
            insert_integer(&mut response_headers, "upload-length", upload.length);
            (StatusCode::OK, response_headers).into_response()
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(error) => {
            tracing::error!(%error, "failed to inspect TUS upload");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

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

async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<HeaderMap, Box<axum::response::Response>> {
    auth::authorize_request(state, headers, None)
        .await
        .map_err(|error| Box::new(error.into_response()))
}

fn validate_version(headers: &HeaderMap) -> Result<(), StatusCode> {
    if headers
        .get("tus-resumable")
        .and_then(|value| value.to_str().ok())
        == Some(TUS_VERSION)
    {
        Ok(())
    } else {
        Err(StatusCode::PRECONDITION_FAILED)
    }
}

fn integer_header(headers: &HeaderMap, name: &'static str) -> Result<u64, StatusCode> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)
}

fn parse_metadata(headers: &HeaderMap) -> Result<BTreeMap<String, String>, StatusCode> {
    let Some(value) = headers.get("upload-metadata") else {
        return Ok(BTreeMap::new());
    };
    let value = value.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
    value
        .split(',')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            let (key, encoded) = entry
                .trim()
                .split_once(' ')
                .ok_or(StatusCode::BAD_REQUEST)?;
            if key.is_empty() {
                return Err(StatusCode::BAD_REQUEST);
            }
            let decoded = STANDARD
                .decode(encoded)
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            let decoded = String::from_utf8(decoded).map_err(|_| StatusCode::BAD_REQUEST)?;
            Ok((key.to_owned(), decoded))
        })
        .collect()
}

fn protocol_headers(mut headers: HeaderMap) -> HeaderMap {
    headers.insert("tus-resumable", HeaderValue::from_static(TUS_VERSION));
    headers.insert("tus-version", HeaderValue::from_static(TUS_VERSION));
    headers.insert("tus-extension", HeaderValue::from_static("creation"));
    headers
}

fn insert_integer(headers: &mut HeaderMap, name: &'static str, value: u64) {
    if let Ok(value) = HeaderValue::from_str(&value.to_string()) {
        headers.insert(name, value);
    }
}
