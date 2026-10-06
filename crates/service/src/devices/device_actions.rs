use std::sync::Arc;

use super::callbacks::callback_client;
use super::*;
use crate::error::ErrorResponse;
use crate::{auth, events::ServerEvent, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

#[utoipa::path(get, path = "/api/v1/device/all/active", tag = "Devices", summary = "List approved active devices", description = "Public compatibility route returning complete approved, non-deleted legacy device rows.", responses((status = 200, description = "Approved devices", body = [Object]), (status = 500, description = "Device query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn active_devices(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.active_devices().await {
        Ok(devices) => Json(devices).into_response(),
        Err(error) => repository_error(error, "Failed to list active devices"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/heartbeat/timeout", tag = "Devices", summary = "Configure device heartbeat timeout", description = "Requires the target device to acknowledge the timeout before the new value is persisted.", security(("bearer_auth" = [])), request_body = HeartbeatTimeoutRequest, responses((status = 200, description = "Heartbeat timeout saved", body = CallbackResponse), (status = 400, description = "Timeout or device address is invalid", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 500, description = "Device lookup or timeout persistence failed", body = ErrorResponse), (status = 502, description = "Device rejected or could not receive the timeout", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn update_heartbeat_timeout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<HeartbeatTimeoutRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    if request.value < 0 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_timeout",
            "Heartbeat timeout must not be negative",
        );
    }
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.device_action_target(&request.device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => return repository_error(error, "Failed to load device"),
    };
    let url = match reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/configure/heartbeat",
        target.ip_address,
    )) {
        Ok(url) => url,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_device_address",
                "Device IP address is invalid",
            );
        }
    };
    let response = callback_client()
        .post(url)
        .json(&serde_json::json!({"timeout": request.value}))
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => {
            tracing::warn!(status = %response.status(), device_id = request.device_id, "heartbeat configuration was rejected");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "device_callback_failed",
                "Device did not acknowledge heartbeat configuration",
            );
        }
        Err(error) => {
            tracing::warn!(%error, device_id = request.device_id, "heartbeat configuration callback failed");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "device_unreachable",
                "Device heartbeat endpoint is unavailable",
            );
        }
    }
    match repository.update_heartbeat_timeout(&request.device_id, request.value).await {
        Ok(true) => (response_headers, Json(serde_json::json!({"success": true, "message": "Heartbeat timeout saved successfully."}))).into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => repository_error(error, "Failed to save heartbeat timeout"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/user", tag = "Devices", summary = "List approved devices for users", description = "Returns the approved, non-deleted device inventory projection used by authenticated user workflows.", params(UserDeviceListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated user device inventory", body = DeviceList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Device query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list_user_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<UserDeviceListQuery>,
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
    match repository.list_user_devices(&query).await {
        Ok(devices) => (response_headers, Json(devices)).into_response(),
        Err(error) => repository_error(error, "Failed to list user devices"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/controller", tag = "Devices", summary = "List device controllers", description = "Returns paginated controller inventory with retained search, status, family, and sort behavior.", params(ControllerListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated controller inventory", body = ControllerList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Controller query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list_controllers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ControllerListQuery>,
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
    match repository.list_controllers(&query).await {
        Ok(controllers) => (response_headers, Json(controllers)).into_response(),
        Err(error) => repository_error(error, "Failed to list device controllers"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/{id}/action", tag = "Devices", summary = "Approve or decline a device request", description = "Approval is committed only after the device acknowledges its expected identity; decline updates persistence directly.", params(("id" = String, Path)), security(("bearer_auth" = [])), request_body = DeviceActionRequest, responses((status = 200, description = "Device request updated", body = DeviceDataResponse), (status = 400, description = "Stored device address is invalid", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 409, description = "Device identity or approval state mismatch", body = ErrorResponse), (status = 500, description = "Device lookup or mutation failed", body = ErrorResponse), (status = 502, description = "Device rejected or could not receive approval", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn device_action(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Json(request): Json<DeviceActionRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
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
    let user_id = auth::token_subject(&headers).unwrap_or_else(|| "system".to_owned());
    let target = match repository.device_action_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => return repository_error(error, "Failed to load device"),
    };
    if request.action == DeviceAction::Approved {
        if target.status != "requested" {
            return error_response(
                StatusCode::CONFLICT,
                "invalid_state",
                "Device is no longer awaiting approval",
            );
        }
        let url = match reqwest::Url::parse(&format!(
            "http://{}:{EDGE_CONTROLLER_PORT}/approve",
            target.ip_address
        )) {
            Ok(url) => url,
            Err(_) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_device_address",
                    "Device IP address is invalid",
                );
            }
        };
        let client = callback_client();
        match client
            .post(url)
            .json(&serde_json::json!({"UID": target.device_id, "IP": target.ip_address}))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) if response.status() == reqwest::StatusCode::CONFLICT => {
                return error_response(
                    StatusCode::CONFLICT,
                    "device_identity_mismatch",
                    "Unable to approve device: IP and MAC address do not match the expected device.",
                );
            }
            Ok(response) => {
                tracing::warn!(status = %response.status(), device_id, "device approval was rejected");
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    "device_callback_failed",
                    "Device did not acknowledge approval",
                );
            }
            Err(error) => {
                tracing::warn!(%error, device_id, "device approval callback failed");
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    "device_unreachable",
                    "Device approval endpoint is unavailable",
                );
            }
        }
    }
    match repository
        .apply_device_action(&device_id, request.action, &user_id)
        .await
    {
        Ok(Some(device)) => {
            if request.action == DeviceAction::Approved
                && let Err(error) = state
                    .event_publisher
                    .publish(ServerEvent::alert("device-approval", device.clone()))
            {
                tracing::warn!(%error, device_id, "failed to publish device approval alert");
            }
            (
                response_headers,
                Json(serde_json::json!({"success": true, "data": device})),
            )
                .into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => repository_error(error, "Failed to update device status"),
    }
}
