use std::sync::Arc;

use super::types::DeviceTypesQuery;
use super::*;
use crate::error::ErrorResponse;
use crate::{auth, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

#[utoipa::path(get, path = "/api/v1/device/", tag = "Devices", summary = "List devices", description = "Returns paginated device inventory with retained filters and role-sensitive visibility for frontend configuration views.", params(DeviceListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated device inventory", body = DeviceList), (status = 401, description = "Access token is missing or invalid", body = ErrorResponse), (status = 500, description = "Device query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DeviceListQuery>,
) -> axum::response::Response {
    let (response_headers, is_admin) =
        match auth::authorize_request(&state, &headers, Some("admin")).await {
            Ok(headers) => (headers, true),
            Err(_) => match auth::authorize_request(&state, &headers, None).await {
                Ok(headers) => (headers, false),
                Err(error) => return error.into_response(),
            },
        };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.list_devices(&query, is_admin).await {
        Ok(devices) => (response_headers, Json(devices)).into_response(),
        Err(error) => repository_error(error, "Failed to list devices"),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/device/families",
    tag = "Devices",
    summary = "List device families",
    description = "Returns deterministic distinct non-null device-family values for frontend filters.",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Distinct device families", body = [String]),
        (status = 401, description = "Access token is missing or invalid", body = ErrorResponse),
        (status = 500, description = "Device family query failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn device_families(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    protected_string_list(&state, response_headers, |repository| async move {
        repository.device_families().await
    })
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/deviceTypes",
    tag = "Devices",
    summary = "List device types",
    description = "Returns deterministic distinct device types, optionally filtered by family; the legacy ALL value disables filtering.",
    params(("deviceFamily" = Option<String>, Query, description = "Family filter; ALL returns every type")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Distinct device types", body = [String]),
        (status = 401, description = "Access token is missing or invalid", body = ErrorResponse),
        (status = 500, description = "Device type query failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn device_types(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DeviceTypesQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    protected_string_list(&state, response_headers, |repository| async move {
        repository
            .device_types(query.device_family.as_deref())
            .await
    })
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/{id}",
    tag = "Devices",
    summary = "Get a device",
    description = "Returns the stable frontend wrapper around the complete device projection, interfaces, and latest execution context.",
    params(("id" = String, Path, description = "Device identifier")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Device with interfaces and latest execution", body = DeviceDataResponse),
        (status = 404, description = "Device not found", body = ErrorResponse),
        (status = 401, description = "Access token is missing or invalid", body = ErrorResponse),
        (status = 500, description = "Device query failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn device_by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.device_by_id(&device_id).await {
        Ok(Some(device)) => (
            response_headers,
            Json(serde_json::json!({"success": true, "data": device})),
        )
            .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => repository_error(error, "Failed to get device"),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/device/{id}/heartbeat",
    tag = "Devices",
    summary = "Get the latest device heartbeat",
    description = "Returns the latest non-disconnected heartbeat for a device or controller in the stable success/data wrapper; missing history yields null data.",
    params(("id" = String, Path, description = "Device or controller identifier")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Latest non-disconnected heartbeat, or null", body = DeviceHeartbeatResponse),
        (status = 401, description = "Access token is missing or invalid", body = ErrorResponse),
        (status = 500, description = "Heartbeat query failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn latest_heartbeat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.latest_heartbeat(&device_id).await {
        Ok(heartbeat) => (
            response_headers,
            Json(serde_json::json!({"success": true, "data": heartbeat})),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to get device heartbeat"),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/device/topology",
    tag = "Devices",
    summary = "List devices for topology",
    description = "Returns lightweight non-deleted device nodes in the stable success/data wrapper used by the topology view.",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Lightweight topology nodes", body = DeviceTopologyResponse),
        (status = 401, description = "Access token is missing or invalid", body = ErrorResponse),
        (status = 500, description = "Topology query failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn device_topology(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.topology().await {
        Ok(devices) => (
            response_headers,
            Json(serde_json::json!({"success": true, "data": devices})),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to get device topology"),
    }
}

async fn protected_string_list<F, Fut>(
    state: &AppState,
    response_headers: HeaderMap,
    operation: F,
) -> axum::response::Response
where
    F: FnOnce(Arc<dyn DeviceRepository>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>, DeviceRepositoryError>>,
{
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match operation(repository).await {
        Ok(values) => (response_headers, Json(values)).into_response(),
        Err(error) => repository_error(error, "Device query failed"),
    }
}
