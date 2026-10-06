use std::sync::Arc;

use super::callbacks::callback_client;
use super::*;
use crate::error::ErrorResponse;
use crate::{auth, events::ServerEvent, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

#[utoipa::path(delete, path = "/api/v1/device/{id}", tag = "Devices", summary = "Delete a device", description = "Requests remote device cleanup when reachable, then applies the retained database deletion and committed inventory event behavior.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Device deleted"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 500, description = "Device deletion failed", body = ErrorResponse), (status = 502, description = "Device did not acknowledge deletion", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn delete_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
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
    let target = match repository.device_delete_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => return repository_error(error, "Failed to load device"),
    };
    let url = match reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/delete",
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
    match callback_client()
        .post(url)
        .json(&serde_json::json!({"UID": device_id}))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => {
            tracing::warn!(status = %response.status(), device_id, "device deletion was rejected");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "device_callback_failed",
                "Device did not acknowledge deletion",
            );
        }
        Err(error) => {
            tracing::warn!(%error, device_id, "device deletion callback failed");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "device_unreachable",
                "Device delete endpoint is unavailable",
            );
        }
    }
    let normalized_family = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .replace(' ', "")
        .to_lowercase();
    if matches!(normalized_family.as_str(), "gen4" | "gen5")
        && let (Some(controller_id), Some(controller_ip)) =
            (&target.controller_id, &target.controller_ip)
    {
        let generation = normalized_family
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u32>()
            .unwrap_or(0);
        if let Ok(url) = reqwest::Url::parse(&format!(
            "http://{controller_ip}:{EDGE_CONTROLLER_PORT}/devCon/delete"
        )) {
            let mut payload =
                serde_json::json!({"uid": controller_id, "gen": generation, "mac": device_id});
            if normalized_family == "gen4" {
                if let Some(serial) = &target.relay_serial {
                    payload["serial"] = serial.clone().into();
                }
                if let Some(channel) = target.relay_channel {
                    payload["channel"] = channel.into();
                }
            }
            if let Err(error) = callback_client().post(url).json(&payload).send().await {
                tracing::warn!(%error, device_id, "controller device-delete notification failed; continuing");
            }
        }
    }
    match repository.delete_device(&device_id).await {
        Ok(true) => (response_headers, Json(serde_json::json!({"success": true, "message": format!("Device {device_id} successfully deleted")}))).into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => repository_error(error, "Failed to delete device"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/controller/{id}", tag = "Devices", summary = "Edit a device controller", description = "Updates mutable controller inventory fields and emits the committed controller-changed event.", params(("id" = String, Path)), security(("bearer_auth" = [])), request_body = ControllerEditRequest, responses((status = 200, description = "Controller updated"), (status = 400, description = "Invalid controller update", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Controller not found", body = ErrorResponse), (status = 500, description = "Controller update failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn edit_controller(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(controller_id): Path<String>,
    Json(request): Json<ControllerEditRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    if request
        .ip_address
        .as_deref()
        .is_some_and(|address| address.trim().is_empty())
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_address",
            "Controller IP address must not be empty",
        );
    }
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.edit_controller(&controller_id, &request).await {
        Ok(Some(controller)) => {
            publish_controller_changed(&state, "updated", &controller_id);
            (
                response_headers,
                Json(serde_json::json!({"success": true, "data": controller})),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Device controller not found",
        ),
        Err(error) => repository_error(error, "Failed to update device controller"),
    }
}

#[utoipa::path(delete, path = "/api/v1/device/controller/{id}", tag = "Devices", summary = "Delete a device controller", description = "Deletes the controller inventory relationship and emits the committed controller-changed event.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 204, description = "Controller deleted"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Controller not found", body = ErrorResponse), (status = 500, description = "Controller deletion failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn delete_controller(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(controller_id): Path<String>,
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
    let target = match repository.controller_delete_target(&controller_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Device controller not found",
            );
        }
        Err(error) => return repository_error(error, "Failed to load device controller"),
    };
    let generation = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse::<u32>()
        .unwrap_or(0);
    if let Ok(url) = reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/devCon/delete",
        target.ip_address
    )) && let Err(error) = callback_client()
        .post(url)
        .json(&serde_json::json!({"uid": controller_id, "gen": generation}))
        .send()
        .await
    {
        tracing::warn!(%error, controller_id, "controller delete notification failed; continuing");
    }
    match repository.delete_controller(&controller_id).await {
        Ok(true) => {
            publish_controller_changed(&state, "deleted", &controller_id);
            (response_headers, StatusCode::NO_CONTENT).into_response()
        }
        Ok(false) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Device controller not found",
        ),
        Err(error) => repository_error(error, "Failed to delete device controller"),
    }
}

pub(super) fn publish_controller_changed(state: &AppState, action: &str, controller_id: &str) {
    if let Err(error) = state.event_publisher.publish(ServerEvent {
        event: "device_controller_changed".to_owned(),
        payload: serde_json::json!({
            "action": action,
            "controllerId": controller_id,
        }),
        room: None,
    }) {
        tracing::warn!(%error, action, controller_id, "failed to publish controller change");
    }
}
