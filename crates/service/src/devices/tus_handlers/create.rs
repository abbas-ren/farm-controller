use std::{path::Path, sync::Arc};

use axum::{
    extract::{Path as AxumPath, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use uuid::Uuid;

use crate::{error::ErrorResponse, state::AppState};

use super::super::{error_response, repository_error, tus_store, upload_events};
use super::*;

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
