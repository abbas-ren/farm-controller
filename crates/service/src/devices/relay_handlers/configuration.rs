//! Modern relay channel configuration and confirmation handlers.
//!
//! Ports Node `configureRelayAndChannels` and `confirmConfigDeviceRelay`
//! (`POST /relay/configure`, `/relay/config/confirmation`).

use crate::socket::types::{
    farm::ServerEvent,
    frontend::{Action, DevicePower, FrontendMessagePayload},
};

use super::*;

/// PURPOSE: Configure relay channels
/// USE: Frontend admin UI (bearer + admin role); also used by workers / other handlers.
#[utoipa::path(post, path = "/api/v1/device/relay/configure", tag = "Devices", summary = "Configure relay channels", description = "Validates and atomically commits one or many channel mappings with clear-first move semantics, then queues hardware synchronization.", request_body = RelayConfigurationRequest, security(("bearer_auth" = [])), responses((status = 202, description = "Configuration committed and hardware sync started", body = RelayConfigurationAccepted), (status = 400, description = "Invalid configuration batch", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay, channel, controller, or device not found", body = ErrorResponse), (status = 409, description = "Device is already mapped", body = ErrorResponse), (status = 500, description = "Relay configuration failed", body = ErrorResponse), (status = 503, description = "Persistence or relay worker unavailable", body = ErrorResponse)))]
pub(crate) async fn configure_relay_channels(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<RelayConfigurationRequest>, JsonRejection>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_handlers::configuration", "configure_relay_channels");
    // Auth: require valid bearer token with administrator role.
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
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => return repository_error(error, "Failed to configure relay channels"),
    };
    let Some(repository) = state.device_repository.clone() else {
        // Guard: device repository is not wired in this process.
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let result = match repository.configure_relay_channels(&configurations).await {
        Ok(result) => result,
        // Map: translate repository failure into the legacy HTTP error envelope.
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
        // Guard: device repository is not wired in this process.
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

/// PURPOSE: Confirm relay configuration
/// USE: publishes Socket.IO frontend events.
#[utoipa::path(post, path = "/api/v1/device/relay/config/confirmation", tag = "Devices", summary = "Confirm relay configuration", description = "Public EdgeController callback that reconciles durable pending relay state before emitting power, progress, and controller events.", request_body = RelayConfigurationConfirmation, responses((status = 200, description = "Confirmation reconciled", body = RelayConfirmationResponse), (status = 400, description = "Invalid callback payload", body = ErrorResponse), (status = 404, description = "Relay confirmation target not found", body = ErrorResponse), (status = 500, description = "Relay confirmation failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn confirm_relay_configuration(
    State(state): State<Arc<AppState>>,
    payload: Result<Json<RelayConfigurationConfirmation>, JsonRejection>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_handlers::configuration", "confirm_relay_configuration");
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
        // Guard: device repository is not wired in this process.
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
                // Publish: notify connected frontend clients via Socket.IO.
                if let Err(error) = state.event_publisher.publish(event) {
                    tracing::warn!(%error, mac = confirmation.mac, "failed to publish relay confirmation");
                }
            }
            Json(response).into_response()
        }
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "Failed to confirm relay configuration"),
    }
}

/// PURPOSE: Confirm relay configuration
/// USE: publishes Socket.IO frontend events.
pub(crate) fn confirmation_events(response: &RelayConfirmationResponse) -> Vec<ServerEvent> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_handlers::configuration", "confirmation_events");
    let Some(relay_id) = response.relay_id else {
        return Vec::new();
    };
    if !response.confirmed {
        return vec![ServerEvent::frontend(
            FrontendMessagePayload::RelayConfigurationStatus {
                relay_id: Some(relay_id.clone()),
                controller_id: response.controller_id.clone(),
                status: "failed".to_owned(),
                total: 1,
                completed: 0,
                errors: vec!["Failed to confirm relay configuration".to_owned()],
                message: Some(response.message.clone()),
            },
        )];
    }
    let mut events = Vec::new();
    if let Some(device_id) = response.device_id.as_deref() {
        events.push(ServerEvent::frontend(
            FrontendMessagePayload::DevicePowerUpdate {
                device_id: device_id.to_owned(),
                power: DevicePower::On,
                changed_at: chrono::Utc::now().to_rfc3339(),
            },
        ));
    }
    if response.configuration_complete {
        events.push(ServerEvent::frontend(
            FrontendMessagePayload::RelayConfigurationStatus {
                relay_id: Some(relay_id.clone()),
                controller_id: response.controller_id.clone(),
                status: "completed".to_owned(),
                total: 1,
                completed: 1,
                errors: Vec::new(),
                message: Some(response.message.clone()),
            },
        ));
        if let Some(controller_id) = response.controller_id.as_deref() {
            events.push(ServerEvent::frontend(
                FrontendMessagePayload::DeviceControllerChanged {
                    action: Action::Updated,
                    controller_id: controller_id.to_owned(),
                },
            ));
        }
    }
    events
}
