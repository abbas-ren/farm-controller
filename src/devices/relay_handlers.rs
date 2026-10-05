use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{auth, error::ErrorResponse, events::ServerEvent, state::AppState};

use super::{
    CallbackStatus, LegacyRelayChannelListQuery, LegacyRelayConfiguration,
    LegacyRelayConfigurationResponse, LegacyRelayConflictQuery, LegacyRelayListQuery,
    LegacyRelayRemapRequest, MessageResponse, RelayChannelRecord, RelayConfigurationAccepted,
    RelayConfigurationConfirmation, RelayConfigurationRequest, RelayConfirmationResponse,
    RelayConflictResponse, RelayIdentityUpdateRequest, RelayIdentityUpdateResponse, RelayRecord,
    constants::{DEVICE_CALLBACK_TIMEOUT, EDGE_CONTROLLER_PORT},
    error_response,
    repository::DeviceRepository,
    repository_error,
    validation::relay_configurations,
};

#[derive(Debug, Deserialize)]
struct RelayStateResponse {
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EdgeRelayIdentityResponse {
    relay: EdgeRelayIdentity,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EdgeRelayIdentity {
    serial_number: String,
    vid: u16,
    pid: u16,
}

fn valid_relay_identity(request: &RelayIdentityUpdateRequest) -> bool {
    let serial_valid = request.serial_number.as_deref().is_none_or(|serial| {
        !serial.is_empty()
            && serial.len() <= 255
            && serial
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b',' | b'"'))
    });
    let vid_pid_valid = {
        let value = request.vid_pid.as_str();
        value.len() == 9
            && value.as_bytes()[4] == b':'
            && value
                .bytes()
                .enumerate()
                .all(|(index, byte)| index == 4 || byte.is_ascii_hexdigit())
    };
    serial_valid && vid_pid_valid
}

#[cfg(test)]
mod identity_validation_tests {
    use super::{RelayIdentityUpdateRequest, valid_relay_identity};

    #[test]
    fn vid_pid_is_required_and_serial_is_optional() {
        assert!(valid_relay_identity(&RelayIdentityUpdateRequest {
            serial_number: None,
            vid_pid: "0403:6001".to_owned(),
        }));
        assert!(valid_relay_identity(&RelayIdentityUpdateRequest {
            serial_number: Some("RELAY-A".to_owned()),
            vid_pid: "0403:6001".to_owned(),
        }));
        assert!(!valid_relay_identity(&RelayIdentityUpdateRequest {
            serial_number: Some("RELAY-A".to_owned()),
            vid_pid: String::new(),
        }));
    }
}

#[utoipa::path(put, path = "/api/v1/device/relay/{id}/identity", tag = "Devices", summary = "Replace a Gen4 relay board identity", description = "Updates EdgeController's active USB relay identity and persists the resolved serial and VID:PID.", params(("id" = Uuid, Path)), request_body = RelayIdentityUpdateRequest, security(("bearer_auth" = [])), responses((status = 200, description = "Relay identity updated", body = RelayIdentityUpdateResponse), (status = 400, description = "Invalid relay identity", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay not found", body = ErrorResponse), (status = 409, description = "Relay serial already assigned", body = ErrorResponse), (status = 502, description = "EdgeController rejected the update", body = ErrorResponse), (status = 503, description = "Persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn update_relay_identity(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(relay_id): Path<Uuid>,
    payload: Result<Json<RelayIdentityUpdateRequest>, JsonRejection>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Json(request) = match payload {
        Ok(payload) if valid_relay_identity(&payload.0) => payload,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "validation",
                "A hexadecimal vidPid is required; serialNumber is optional",
            );
        }
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.relay_identity_target(relay_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "relay_not_found", "Relay not found");
        }
        Err(error) => return repository_error(error, "Failed to load relay identity target"),
    };
    let url = match reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/relay/identity",
        target.controller_ip
    )) {
        Ok(url) => url,
        Err(_) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                "controller_address",
                "Invalid EdgeController address",
            );
        }
    };
    let client = match crate::external_http::client(std::time::Duration::from_secs(10)) {
        Ok(client) => client,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "http_client",
                &error.to_string(),
            );
        }
    };
    let edge = match client.post(url).json(&request).send().await {
        Ok(response) if response.status().is_success() => {
            match response.json::<EdgeRelayIdentityResponse>().await {
                Ok(response) => response,
                Err(error) => {
                    return error_response(
                        StatusCode::BAD_GATEWAY,
                        "controller_response",
                        &format!("Invalid EdgeController response: {error}"),
                    );
                }
            }
        }
        Ok(response) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                "controller_rejected",
                &format!(
                    "EdgeController rejected relay identity with {}",
                    response.status()
                ),
            );
        }
        Err(error) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                "controller_unreachable",
                &format!("Could not update EdgeController relay identity: {error}"),
            );
        }
    };
    match repository
        .update_relay_identity(
            relay_id,
            &target.current_serial,
            &edge.relay.serial_number,
            &format!("{:04X}", edge.relay.vid),
            &format!("{:04X}", edge.relay.pid),
        )
        .await
    {
        Ok(true) => {
            super::publish_controller_changed(&state, "updated", &target.controller_id);
            (
                response_headers,
                Json(RelayIdentityUpdateResponse {
                    message: "Relay identity updated",
                    relay_id,
                    serial_number: edge.relay.serial_number,
                    vid_pid: format!("{:04X}:{:04X}", edge.relay.vid, edge.relay.pid),
                }),
            )
                .into_response()
        }
        Ok(false) => error_response(StatusCode::NOT_FOUND, "relay_not_found", "Relay not found"),
        Err(error) => repository_error(error, "Failed to persist relay identity"),
    }
}

async fn legacy_repository(
    state: &Arc<AppState>,
    headers: &HeaderMap,
) -> Result<(HeaderMap, Arc<dyn DeviceRepository>), Box<axum::response::Response>> {
    let response_headers = auth::authorize_request(state, headers, Some("admin"))
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

#[utoipa::path(post, path = "/api/v1/device/relay/config", tag = "Devices", summary = "Configure one relay channel using the retained legacy contract", description = "Commits legacy single-channel mapping and automatic prior-device unmapping, then queues controller hardware synchronization.", request_body = LegacyRelayConfiguration, security(("bearer_auth" = [])), responses((status = 200, description = "Device configured with relay", body = LegacyRelayConfigurationResponse), (status = 400, description = "Invalid relay configuration", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay dependency not found", body = ErrorResponse), (status = 500, description = "Relay configuration failed", body = ErrorResponse), (status = 503, description = "Persistence or relay worker unavailable", body = ErrorResponse)))]
pub(crate) async fn configure_legacy_relay(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<LegacyRelayConfiguration>, JsonRejection>,
) -> axum::response::Response {
    let (response_headers, repository) = match legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "validation",
                "Invalid relay configuration request body",
            );
        }
    };
    match repository.configure_legacy_relay(&request).await {
        Ok(result) => {
            let data = result
                .channels
                .into_iter()
                .next()
                .unwrap_or(serde_json::Value::Null);
            if let Err(error) = state
                .relay_sync
                .enqueue(result.actions, Some(request.relay_id))
            {
                return error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "worker_unavailable",
                    &error,
                );
            }
            (
                response_headers,
                Json(serde_json::json!({
                    "message": "Device configured with relay successfully",
                    "data": data,
                })),
            )
                .into_response()
        }
        Err(error) => repository_error(error, "Failed to configure device relay"),
    }
}

#[utoipa::path(post, path = "/api/v1/device/relay/config/fresh", tag = "Devices", summary = "Remove stale duplicate relay instances", description = "Hard-cleans stale duplicate relay/channel rows for the retained legacy recovery workflow.", request_body = LegacyRelayConfiguration, security(("bearer_auth" = [])), responses((status = 200, description = "Duplicate relay instances removed", body = MessageResponse), (status = 400, description = "Invalid request", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay not found", body = ErrorResponse), (status = 500, description = "Relay cleanup failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn fresh_legacy_relay(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<LegacyRelayConfiguration>, JsonRejection>,
) -> axum::response::Response {
    let (response_headers, repository) = match legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "validation",
                "Invalid relay configuration request body",
            );
        }
    };
    match repository.fresh_legacy_relay(request.relay_id).await {
        Ok(()) => (
            response_headers,
            Json(serde_json::json!({
                "message": "Device configured with relay successfully",
            })),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to clean existing relay mappings"),
    }
}

#[utoipa::path(post, path = "/api/v1/device/relay/config/remap", tag = "Devices", summary = "Remap channels from a duplicate relay instance", description = "Moves retained channel mappings onto the surviving relay in one transaction, then queues controller resynchronization.", request_body = LegacyRelayRemapRequest, security(("bearer_auth" = [])), responses((status = 200, description = "Relay channels remapped", body = MessageResponse), (status = 400, description = "Invalid relay id", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay or controller not found", body = ErrorResponse), (status = 500, description = "Relay remapping failed", body = ErrorResponse), (status = 503, description = "Persistence or relay worker unavailable", body = ErrorResponse)))]
pub(crate) async fn remap_legacy_relay(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<LegacyRelayRemapRequest>, JsonRejection>,
) -> axum::response::Response {
    let (response_headers, repository) = match legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "validation",
                "Invalid or missing relayId in request body",
            );
        }
    };
    let relay_id = match Uuid::parse_str(&request.relay_id) {
        Ok(relay_id) => relay_id,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "validation",
                "Invalid or missing relayId in request body",
            );
        }
    };
    match repository.remap_legacy_relay(relay_id).await {
        Ok(result) => {
            if let Err(error) = state.relay_sync.enqueue(result.actions, Some(relay_id)) {
                return error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "worker_unavailable",
                    &error,
                );
            }
            (
                response_headers,
                Json(serde_json::json!({
                    "message": "Device relay remapped successfully",
                })),
            )
                .into_response()
        }
        Err(error) => repository_error(error, "Failed to remap relay channels"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/relay/check-conflicts", tag = "Devices", summary = "Check retained relay mapping conflicts", description = "Checks device, relay, and channel occupancy using retained cleanup semantics and reports one compatibility conflict flag.", params(LegacyRelayConflictQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Relay conflict state", body = RelayConflictResponse), (status = 400, description = "Invalid query", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay not found", body = ErrorResponse), (status = 500, description = "Relay conflict query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn legacy_relay_conflicts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LegacyRelayConflictQuery>,
) -> axum::response::Response {
    let (response_headers, repository) = match legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.legacy_relay_conflicts(&query).await {
        Ok(conflict) => (
            response_headers,
            Json(serde_json::json!({"conflict": conflict})),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to check relay channel conflicts"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/relay/", tag = "Devices", summary = "List relays using the retained legacy contract", description = "Lists non-deleted relays with optional controller filtering and retained legacy row projections.", params(LegacyRelayListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Relays", body = Vec<RelayRecord>), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Relay query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn legacy_relays(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LegacyRelayListQuery>,
) -> axum::response::Response {
    let (response_headers, repository) = match legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.legacy_relays(&query).await {
        Ok(relays) => (response_headers, Json(relays)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch relays"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/relay/channel", tag = "Devices", summary = "List relay channels using the retained legacy contract", description = "Lists non-deleted relay channels with optional device or relay filters and attached compatibility data.", params(LegacyRelayChannelListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Relay channels", body = Vec<RelayChannelRecord>), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Relay channel query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn legacy_relay_channels(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LegacyRelayChannelListQuery>,
) -> axum::response::Response {
    let (response_headers, repository) = match legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.legacy_relay_channels(&query).await {
        Ok(channels) => (response_headers, Json(channels)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch relay channels"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/relay/channel/{id}", tag = "Devices", summary = "Get a retained relay channel", description = "Returns one non-deleted retained relay-channel record by UUID.", params(("id" = Uuid, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Relay channel", body = RelayChannelRecord), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay channel not found", body = ErrorResponse), (status = 500, description = "Relay channel query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn legacy_relay_channel_by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(channel_id): Path<Uuid>,
) -> axum::response::Response {
    let (response_headers, repository) = match legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.legacy_relay_channel_by_id(channel_id).await {
        Ok(Some(channel)) => (response_headers, Json(channel)).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "relay_channel_not_found",
            "Relay channel not found",
        ),
        Err(error) => repository_error(error, "Failed to fetch relay channel"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/relay/{id}", tag = "Devices", summary = "Get a retained relay", description = "Returns one non-deleted retained relay with its compatibility projection by UUID.", params(("id" = Uuid, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Relay", body = RelayRecord), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay not found", body = ErrorResponse), (status = 500, description = "Relay query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn legacy_relay_by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(relay_id): Path<Uuid>,
) -> axum::response::Response {
    let (response_headers, repository) = match legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.legacy_relay_by_id(relay_id).await {
        Ok(Some(relay)) => (response_headers, Json(relay)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "relay_not_found", "Relay not found"),
        Err(error) => repository_error(error, "Failed to fetch relay"),
    }
}

#[utoipa::path(delete, path = "/api/v1/device/relay/channel/{id}", tag = "Devices", summary = "Soft-delete a retained relay channel", description = "Applies the retained paranoid soft-delete behavior so historical channel rows remain recoverable.", params(("id" = Uuid, Path)), security(("bearer_auth" = [])), responses((status = 204, description = "Relay channel deleted"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay channel not found", body = ErrorResponse), (status = 500, description = "Relay channel deletion failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn delete_legacy_relay_channel(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(channel_id): Path<Uuid>,
) -> axum::response::Response {
    let (response_headers, repository) = match legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.delete_legacy_relay_channel(channel_id).await {
        Ok(true) => (StatusCode::NO_CONTENT, response_headers).into_response(),
        Ok(false) => error_response(
            StatusCode::NOT_FOUND,
            "relay_channel_not_found",
            "Relay channel not found",
        ),
        Err(error) => repository_error(error, "Failed to delete relay channel"),
    }
}

#[utoipa::path(delete, path = "/api/v1/device/relay/{id}", tag = "Devices", summary = "Soft-delete a retained relay", description = "Applies the retained paranoid soft-delete behavior so historical relay rows remain recoverable.", params(("id" = Uuid, Path)), security(("bearer_auth" = [])), responses((status = 204, description = "Relay deleted"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay not found", body = ErrorResponse), (status = 500, description = "Relay deletion failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn delete_legacy_relay(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(relay_id): Path<Uuid>,
) -> axum::response::Response {
    let (response_headers, repository) = match legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.delete_legacy_relay(relay_id).await {
        Ok(true) => (StatusCode::NO_CONTENT, response_headers).into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "relay_not_found", "Relay not found"),
        Err(error) => repository_error(error, "Failed to delete relay"),
    }
}

#[utoipa::path(post, path = "/api/v1/device/relay/configure", tag = "Devices", summary = "Configure relay channels", description = "Validates and atomically commits one or many channel mappings with clear-first move semantics, then queues hardware synchronization.", request_body = RelayConfigurationRequest, security(("bearer_auth" = [])), responses((status = 202, description = "Configuration committed and hardware sync started", body = RelayConfigurationAccepted), (status = 400, description = "Invalid configuration batch", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay, channel, controller, or device not found", body = ErrorResponse), (status = 409, description = "Device is already mapped", body = ErrorResponse), (status = 500, description = "Relay configuration failed", body = ErrorResponse), (status = 503, description = "Persistence or relay worker unavailable", body = ErrorResponse)))]
pub(crate) async fn configure_relay_channels(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<RelayConfigurationRequest>, JsonRejection>,
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
                "Invalid relay configuration request body",
            );
        }
    };
    let (configurations, is_batch) = match relay_configurations(request) {
        Ok(configurations) => configurations,
        Err(error) => return repository_error(error, "Failed to configure relay channels"),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let result = match repository.configure_relay_channels(&configurations).await {
        Ok(result) => result,
        Err(error) => return repository_error(error, "Failed to configure relay channels"),
    };
    let relay_id = configurations
        .first()
        .map(|configuration| configuration.relay_id);
    let data = if is_batch {
        serde_json::Value::Array(result.channels)
    } else {
        result
            .channels
            .into_iter()
            .next()
            .unwrap_or(serde_json::Value::Null)
    };
    if let Err(error) = state.relay_sync.enqueue(result.actions, relay_id) {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "worker_unavailable",
            &error,
        );
    }
    (
        StatusCode::ACCEPTED,
        response_headers,
        Json(RelayConfigurationAccepted {
            message: "Relay channel(s) configuration accepted. Hardware sync in progress.",
            data,
        }),
    )
        .into_response()
}

#[utoipa::path(post, path = "/api/v1/device/relay/config/confirmation", tag = "Devices", summary = "Confirm relay configuration", description = "Public EdgeController callback that reconciles durable pending relay state before emitting power, progress, and controller events.", request_body = RelayConfigurationConfirmation, responses((status = 200, description = "Confirmation reconciled", body = RelayConfirmationResponse), (status = 400, description = "Invalid callback payload", body = ErrorResponse), (status = 404, description = "Relay confirmation target not found", body = ErrorResponse), (status = 500, description = "Relay confirmation failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn confirm_relay_configuration(
    State(state): State<Arc<AppState>>,
    payload: Result<Json<RelayConfigurationConfirmation>, JsonRejection>,
) -> axum::response::Response {
    let Json(confirmation) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "validation",
                "Missing or invalid relay configuration confirmation",
            );
        }
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository
        .confirm_relay_configuration(
            &confirmation.mac,
            confirmation.status == CallbackStatus::Success,
        )
        .await
    {
        Ok(response) => {
            for event in confirmation_events(&response) {
                if let Err(error) = state.event_publisher.publish(event) {
                    tracing::warn!(%error, mac = confirmation.mac, "failed to publish relay confirmation");
                }
            }
            Json(response).into_response()
        }
        Err(error) => repository_error(error, "Failed to confirm relay configuration"),
    }
}

pub(crate) fn confirmation_events(response: &super::RelayConfirmationResponse) -> Vec<ServerEvent> {
    let Some(relay_id) = response.relay_id else {
        return Vec::new();
    };
    if !response.confirmed {
        return vec![ServerEvent {
            event: "relay_configuration_status".to_owned(),
            payload: serde_json::json!({
                "controllerId": response.controller_id.as_deref(),
                "relayId": relay_id,
                "status": "failed",
                "total": 1,
                "completed": 0,
                "message": response.message.as_str(),
            }),
            room: None,
        }];
    }
    let mut events = Vec::new();
    if let Some(device_id) = response.device_id.as_deref() {
        events.push(ServerEvent {
            event: "device_power_update".to_owned(),
            payload: serde_json::json!({
                "deviceId": device_id,
                "power": "on",
                "changedAt": chrono::Utc::now().to_rfc3339(),
            }),
            room: None,
        });
    }
    if response.configuration_complete {
        events.push(ServerEvent {
            event: "relay_configuration_status".to_owned(),
            payload: serde_json::json!({
                "controllerId": response.controller_id.as_deref(),
                "relayId": relay_id,
                "status": "completed",
                "total": 1,
                "completed": 1,
            }),
            room: None,
        });
        if let Some(controller_id) = response.controller_id.as_deref() {
            events.push(ServerEvent {
                event: "device_controller_changed".to_owned(),
                payload: serde_json::json!({
                    "action": "updated",
                    "controllerId": controller_id,
                }),
                room: None,
            });
        }
    }
    events
}

#[utoipa::path(get, path = "/api/v1/device/relay/state/{id}", tag = "Devices", summary = "Read device relay state", description = "Queries the mapped EdgeController for the live channel state, validates the response, then persists the observed state.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Current relay state", body = String), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay mapping not found", body = ErrorResponse), (status = 500, description = "Stored mapping or persistence failed", body = ErrorResponse), (status = 502, description = "Controller state request failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn relay_device_state(
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
    let target = match repository.relay_state_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "relay_mapping_not_found",
                "Relay channel is not linked to this device",
            );
        }
        Err(error) => return repository_error(error, "Failed to load relay mapping"),
    };
    let Ok(channel) = u8::try_from(target.channel_number) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_relay_mapping",
            "Stored relay channel is outside the supported range",
        );
    };
    if channel > 7 {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_relay_mapping",
            "Stored relay channel is outside the supported range",
        );
    }
    let url = match reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/relay/status",
        target.controller_ip
    )) {
        Ok(url) => url,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid_controller_address",
                "Controller IP address is invalid",
            );
        }
    };
    let response = crate::external_http::client(DEVICE_CALLBACK_TIMEOUT)
        .expect("static relay callback HTTP policy is valid")
        .post(url)
        .json(&serde_json::json!({
            "serial": target.relay_serial,
            "channel": channel,
        }))
        .send()
        .await;
    let relay_state = match response {
        Ok(response) if response.status().is_success() => {
            match response.json::<RelayStateResponse>().await {
                Ok(response) if matches!(response.state.as_str(), "on" | "off") => response.state,
                _ => {
                    return error_response(
                        StatusCode::BAD_GATEWAY,
                        "invalid_relay_response",
                        "Controller returned an invalid relay state",
                    );
                }
            }
        }
        Ok(response) => {
            tracing::warn!(status = %response.status(), device_id, "controller rejected relay state request");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "relay_state_failed",
                "Controller rejected the relay state request",
            );
        }
        Err(error) => {
            tracing::warn!(%error, device_id, "controller relay state request failed");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "controller_unreachable",
                "Controller relay endpoint is unavailable",
            );
        }
    };
    if let Err(error) = repository
        .update_relay_state(target.channel_id, &relay_state)
        .await
    {
        return repository_error(error, "Failed to save relay state");
    }
    (response_headers, Json(relay_state)).into_response()
}
