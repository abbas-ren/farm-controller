use std::{path::Path, sync::Arc};

use axum::{
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use uuid::Uuid;

use crate::{error::ErrorResponse, state::AppState};

use super::{super::tus_store, *};

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
