use super::*;

#[utoipa::path(post, path = "/api/v1/device/relay/config", tag = "Devices", summary = "Configure one relay channel using the retained legacy contract", description = "Commits legacy single-channel mapping and automatic prior-device unmapping, then queues controller hardware synchronization.", request_body = LegacyRelayConfiguration, security(("bearer_auth" = [])), responses((status = 200, description = "Device configured with relay", body = LegacyRelayConfigurationResponse), (status = 400, description = "Invalid relay configuration", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay dependency not found", body = ErrorResponse), (status = 500, description = "Relay configuration failed", body = ErrorResponse), (status = 503, description = "Persistence or relay worker unavailable", body = ErrorResponse)))]
pub(crate) async fn configure_legacy_relay(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<LegacyRelayConfiguration>, JsonRejection>,
) -> axum::response::Response {
    let (response_headers, repository) = match super::legacy_repository(&state, &headers).await {
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
    let (response_headers, repository) = match super::legacy_repository(&state, &headers).await {
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
    let (response_headers, repository) = match super::legacy_repository(&state, &headers).await {
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
    let (response_headers, repository) = match super::legacy_repository(&state, &headers).await {
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
    let (response_headers, repository) = match super::legacy_repository(&state, &headers).await {
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
    let (response_headers, repository) = match super::legacy_repository(&state, &headers).await {
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
    let (response_headers, repository) = match super::legacy_repository(&state, &headers).await {
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
    let (response_headers, repository) = match super::legacy_repository(&state, &headers).await {
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
    let (response_headers, repository) = match super::legacy_repository(&state, &headers).await {
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
    let (response_headers, repository) = match super::legacy_repository(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.delete_legacy_relay(relay_id).await {
        Ok(true) => (StatusCode::NO_CONTENT, response_headers).into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "relay_not_found", "Relay not found"),
        Err(error) => repository_error(error, "Failed to delete relay"),
    }
}
