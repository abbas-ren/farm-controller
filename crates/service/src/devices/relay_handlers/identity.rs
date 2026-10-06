use super::*;

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
    let client = match crate::external_http::client(RELAY_IDENTITY_TIMEOUT) {
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
            super::super::publish_controller_changed(&state, "updated", &target.controller_id);
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
