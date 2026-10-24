//! UART topology verification and persistence handlers.
//!
//! Ports Node UART configuration flows exposed as `POST /api/v1/device/uart/configure`
//! (see modern relay/UART UI). Admin frontend verifies UART with EdgeController then
//! persists the mapping.

use std::sync::Arc;

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Serialize;
use uuid::Uuid;

use crate::{auth, error::ErrorResponse, state::AppState};

use super::{
    EDGE_CONTROLLER_PORT, UART_CONFIGURATION_TIMEOUT, UartConfigurationRequest,
    UartConfigurationResponse, error_response,
};

struct UartContext {
    device_id: String,
    device_mac: String,
    generation: i16,
    controller_address: String,
    relay_serial: Option<String>,
    channel_number: Option<i32>,
    relay_channel_id: Option<Uuid>,
    relay_confirmed: bool,
}

#[derive(Debug)]
enum UartLoadError {
    NotFound,
    Database(sea_orm::DbErr),
}

impl From<sea_orm::DbErr> for UartLoadError {
/// PURPOSE: Map SeaORM failures into UART context load errors.
/// USE: load_context database lookups.
    fn from(error: sea_orm::DbErr) -> Self {
        Self::Database(error)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EdgeUartRequest<'a> {
    mac: &'a str,
    #[serde(rename = "gen")]
    generation: u8,
    vid_pid: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    serial: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<u8>,
}

/// PURPOSE: Normalize USB VID/PID strings for UART matching.
/// USE: UART configure validation.
fn normalized_vid_pid(value: &str) -> Option<String> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::uart_handlers", "normalized_vid_pid");
    let value = value.trim().to_ascii_lowercase();
    let (vid, pid) = value.split_once(':')?;
    (vid.len() == 4
        && pid.len() == 4
        && vid
            .bytes()
            .chain(pid.bytes())
            .all(|byte| byte.is_ascii_hexdigit()))
    .then_some(value)
}

/// PURPOSE: Normalize MAC address formatting for UART lookups.
/// USE: UART configure validation.
fn normalize_mac(value: &str) -> String {
    value.to_ascii_lowercase().replace([':', '-'], "")
}

/// PURPOSE: Infer device generation from family/type strings.
/// USE: UART configure prerequisite checks.
fn detect_generation(
    device_family: Option<&str>,
    device_type: Option<&str>,
    controller_family: Option<&str>,
) -> i16 {
    let blob = format!(
        "{} {} {}",
        device_family.unwrap_or_default(),
        device_type.unwrap_or_default(),
        controller_family.unwrap_or_default()
    )
    .to_ascii_lowercase()
    .replace(' ', "");
    if blob.contains("gen3") {
        3
    } else if blob.contains("gen4") {
        4
    } else if blob.contains("gen5") {
        5
    } else {
        0
    }
}

/// PURPOSE: Load device/controller/relay context required for UART configure.
/// USE: configure_uart handler.
async fn load_context(
    state: &AppState,
    request: &UartConfigurationRequest,
) -> Result<UartContext, UartLoadError> {
    use crate::persistence::entity::{
        device, device_controller, pending_relay_config, relay, relay_channel,
    };
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::uart_handlers", "load_context");
    let database = state
        .database
        .as_ref()
        .expect("database checked by handler");
    let orm = database.orm();
    let device_id = request.device_id.trim();
    let controller_id = request.controller_id.trim();

    let device_model = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .one(orm)
        .await?
        .ok_or(UartLoadError::NotFound)?;
    let controller = device_controller::Entity::find()
        .filter(device_controller::Column::DeviceControllerId.eq(controller_id))
        .filter(device_controller::Column::DeletedAt.is_null())
        .one(orm)
        .await?
        .ok_or(UartLoadError::NotFound)?;

    // Prefer stored MAC; fall back to device_id when MAC was never populated (legacy rows).
    let device_mac = device_model
        .mac_address
        .as_deref()
        .filter(|mac| !mac.is_empty())
        .unwrap_or(device_model.device_id.as_str())
        .to_owned();
    let generation = detect_generation(
        device_model.device_family.as_deref(),
        device_model.device_type.as_deref(),
        controller.device_family.as_deref(),
    );

    let mut relay_serial = None;
    let mut channel_number = None;
    let mut relay_channel_id = None;
    let mut relay_confirmed = false;

    if let Some(relay_id) = request.relay_id {
        let relay_model = relay::Entity::find_by_id(relay_id)
            .filter(relay::Column::DeviceControllerId.eq(controller_id))
            .filter(relay::Column::DeletedAt.is_null())
            .one(orm)
            .await?;
        if let Some(relay_model) = relay_model {
            relay_serial = Some(relay_model.serial_number);
            if let Some(channel_id) = request.channel_id {
                let channel = relay_channel::Entity::find_by_id(channel_id)
                    .filter(relay_channel::Column::RelayId.eq(relay_id))
                    .filter(relay_channel::Column::DeviceId.eq(device_id))
                    .filter(relay_channel::Column::DeletedAt.is_null())
                    .one(orm)
                    .await?;
                if let Some(channel) = channel {
                    channel_number = Some(channel.channel_number);
                    relay_channel_id = Some(channel.id);
                    // Gen3/Gen4 require a Confirmed pending_relay_config for this MAC+channel
                    // (physical GPIO probe completed) before UART configure is allowed.
                    let normalized = normalize_mac(&device_mac);
                    relay_confirmed = pending_relay_config::Entity::find()
                        .filter(pending_relay_config::Column::RelayChannelId.eq(channel.id))
                        .order_by_desc(pending_relay_config::Column::CreatedAt)
                        .all(orm)
                        .await?
                        .into_iter()
                        .find(|pending| normalize_mac(&pending.device_mac) == normalized)
                        .is_some_and(|pending| {
                            matches!(
                                pending.status,
                                pending_relay_config::PendingRelayConfigStatus::Confirmed
                            )
                        });
                }
            }
        }
    }

    Ok(UartContext {
        device_id: device_model.device_id,
        device_mac,
        generation,
        controller_address: controller.ip_address,
        relay_serial,
        channel_number,
        relay_channel_id,
        relay_confirmed,
    })
}

/// PURPOSE: Enforce generation-specific UART prerequisites (relay confirm vs Gen5 CPLD).
/// USE: configure_uart after load_context, before calling EdgeController.
fn validate_context(
    context: &UartContext,
    request: &UartConfigurationRequest,
) -> Result<(u8, Option<u8>), &'static str> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::uart_handlers", "validate_context");
    let generation = u8::try_from(context.generation)
        .ok()
        .filter(|generation| matches!(generation, 3..=5))
        .ok_or("Device generation must be Gen3, Gen4, or Gen5")?;
    match generation {
        3 | 4 => {
            if request.relay_id.is_none() || request.channel_id.is_none() {
                return Err("Gen3/Gen4 UART configuration requires relay and channel IDs");
            }
            if context.relay_serial.is_none()
                || context.relay_channel_id != request.channel_id
                || !context.relay_confirmed
            {
                return Err(
                    "Relay channel and GPIO must be physically confirmed before UART configuration",
                );
            }
            let channel = context
                .channel_number
                .and_then(|channel| u8::try_from(channel).ok())
                .filter(|channel| *channel <= 7)
                .ok_or("Confirmed relay channel is invalid")?;
            Ok((generation, Some(channel)))
        }
        5 => {
            if request.relay_id.is_some() || request.channel_id.is_some() {
                return Err("Gen5 UART configuration does not accept relay or GPIO information");
            }
            Ok((generation, None))
        }
        _ => unreachable!(),
    }
}

#[utoipa::path(post, path = "/api/v1/device/uart/configure", tag = "Devices", summary = "Verify and configure device UART", description = "Requires confirmed relay/GPIO control for Gen3/Gen4 or CPLD power control for Gen5, waits for EdgeController USB disconnect/reconnect verification, and persists the resolved UART topology.", request_body = UartConfigurationRequest, security(("bearer_auth" = [])), responses((status = 200, description = "UART physically verified and persisted", body = UartConfigurationResponse), (status = 400, description = "Invalid configuration or unconfirmed prerequisite", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Device or controller not found", body = ErrorResponse), (status = 502, description = "EdgeController rejected UART verification", body = ErrorResponse), (status = 503, description = "Persistence unavailable", body = ErrorResponse)))]
/// PURPOSE: Verify and configure device UART via EdgeController, then persist mapping.
/// USE: Frontend admin UI (bearer + admin role); talks to EdgeController over HTTP.
pub(crate) async fn configure_uart(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<UartConfigurationRequest>, JsonRejection>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::uart_handlers", "configure_uart");
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
                "Invalid UART configuration request body",
            );
        }
    };
    let Some(vid_pid) = normalized_vid_pid(&request.uart_vid_pid) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "UART VID:PID must use four hex digits on each side",
        );
    };
    if state.database.is_none() {
        // Guard: device repository is not wired in this process.
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    }
    let context = match load_context(&state, &request).await {
        Ok(context) => context,
        Err(UartLoadError::NotFound) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Device or controller was not found",
            );
        }
        Err(UartLoadError::Database(error)) => {
            tracing::error!(%error, "failed to load UART configuration context");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "persistence",
                "Failed to load UART configuration context",
            );
        }
    };
    let (generation, channel) = match validate_context(&context, &request) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, "validation", message),
    };
    let url = format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/uart/config",
        context.controller_address
    );
    let client = match reqwest::Client::builder()
        .timeout(UART_CONFIGURATION_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "http_client",
                &error.to_string(),
            );
        }
    };
    let edge_request = EdgeUartRequest {
        mac: &context.device_mac,
        generation,
        vid_pid: &vid_pid,
        serial: context.relay_serial.as_deref().filter(|_| generation != 5),
        channel,
    };
    let edge_response = match client.post(url).json(&edge_request).send().await {
        Ok(response) => response,
        Err(error) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                "edge_controller",
                &format!("EdgeController UART request failed: {error}"),
            );
        }
    };
    let status = edge_response.status();
    let body = match edge_response.text().await {
        Ok(body) => body,
        Err(error) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                "edge_controller",
                &format!("Failed to read EdgeController response: {error}"),
            );
        }
    };
    if !status.is_success() {
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("EdgeController rejected UART verification with {status}"));
        return error_response(StatusCode::BAD_GATEWAY, "edge_controller", &message);
    }
    let verified = match serde_json::from_str::<UartConfigurationResponse>(&body) {
        Ok(response) if response.verified && response.generation == generation => response,
        Ok(_) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                "edge_controller",
                "EdgeController returned an unverified or mismatched UART result",
            );
        }
        Err(error) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                "edge_controller",
                &format!("Invalid EdgeController UART response: {error}"),
            );
        }
    };

    let database = state.database.as_ref().expect("database checked above");
    use crate::persistence::entity::{device_controller, device_uart_mapping};
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait,
        sea_query::OnConflict,
    };
    let transaction = match database.orm().begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "persistence",
                &error.to_string(),
            );
        }
    };
    let now = chrono::Utc::now();
    let mapping_model = device_uart_mapping::ActiveModel {
        id: Set(Uuid::new_v4()),
        device_id: Set(context.device_id.clone()),
        controller_id: Set(request.controller_id.trim().to_owned()),
        relay_channel_id: Set(context.relay_channel_id),
        device_mac: Set(context.device_mac.clone()),
        generation: Set(i16::from(generation)),
        requested_vid_pid: Set(verified.vid_pid.clone()),
        tty: Set(verified.tty.clone()),
        usb_serial: Set(verified.usb_serial.clone()),
        interface: Set(i16::from(verified.interface)),
        topology: Set(verified.topology.clone()),
        connection: Set(verified.connection.clone()),
        verified: Set(true),
        verified_at: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
    };
    if let Err(error) = device_uart_mapping::Entity::insert(mapping_model)
        .on_conflict(
            OnConflict::column(device_uart_mapping::Column::DeviceId)
                .update_columns([
                    device_uart_mapping::Column::ControllerId,
                    device_uart_mapping::Column::RelayChannelId,
                    device_uart_mapping::Column::DeviceMac,
                    device_uart_mapping::Column::Generation,
                    device_uart_mapping::Column::RequestedVidPid,
                    device_uart_mapping::Column::Tty,
                    device_uart_mapping::Column::UsbSerial,
                    device_uart_mapping::Column::Interface,
                    device_uart_mapping::Column::Topology,
                    device_uart_mapping::Column::Connection,
                    device_uart_mapping::Column::Verified,
                    device_uart_mapping::Column::VerifiedAt,
                    device_uart_mapping::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&transaction)
        .await
    {
        tracing::error!(%error, "failed to persist UART mapping");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "persistence",
            "Failed to persist verified UART mapping",
        );
    }
    let normalized_mac = normalize_mac(&context.device_mac);
    let patch = serde_json::json!({
        normalized_mac: {
            "uart": verified.tty,
            "uartVidPid": verified.vid_pid,
            "uartTopology": verified.topology,
            "uartConnection": verified.connection,
            "generation": generation
        }
    });
    let controller_update = async {
        let Some(controller) = device_controller::Entity::find()
            .filter(
                device_controller::Column::DeviceControllerId
                    .eq(request.controller_id.trim()),
            )
            .one(&transaction)
            .await?
        else {
            return Ok::<_, sea_orm::DbErr>(());
        };
        let mut mappings = if controller.mappings.is_null() {
            serde_json::json!({})
        } else {
            controller.mappings.clone()
        };
        if let (Some(obj), Some(patch_obj)) = (mappings.as_object_mut(), patch.as_object()) {
            for (key, value) in patch_obj {
                obj.insert(key.clone(), value.clone());
            }
        }
        let mut active: device_controller::ActiveModel = controller.into();
        active.mappings = Set(mappings);
        active.updated_at = Set(now);
        active.update(&transaction).await?;
        Ok(())
    };
    if let Err(error) = controller_update.await {
        tracing::error!(%error, "failed to update controller UART mapping");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "persistence",
            "Failed to update controller UART mapping",
        );
    }
    if let Err(error) = transaction.commit().await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "persistence",
            &error.to_string(),
        );
    }

    (StatusCode::OK, response_headers, Json(verified)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

/// PURPOSE: Build a UartContext fixture with the given generation.
/// USE: uart_handlers validation tests.
    fn context(generation: i16) -> UartContext {
        UartContext {
            device_id: "device-1".into(),
            device_mac: "aabbccddeeff".into(),
            generation,
            controller_address: "192.0.2.10".into(),
            relay_serial: None,
            channel_number: None,
            relay_channel_id: None,
            relay_confirmed: false,
        }
    }

/// PURPOSE: Build a baseline UartConfigurationRequest fixture.
/// USE: uart_handlers validation tests.
    fn request() -> UartConfigurationRequest {
        let _api_trace = crate::observability::ApiTrace::enter("devices::uart_handlers", "request");
        UartConfigurationRequest {
            controller_id: "controller-1".into(),
            device_id: "device-1".into(),
            relay_id: None,
            channel_id: None,
            uart_vid_pid: "0403:6010".into(),
        }
    }

    #[test]
/// PURPOSE: Assert VID:PID normalization requires two 4-digit hex halves.
/// USE: regression for UART configure input validation.
    fn vid_pid_is_normalized_and_strictly_validated() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::uart_handlers", "vid_pid_is_normalized_and_strictly_validated");
        assert_eq!(
            normalized_vid_pid(" 0403:AbCd ").as_deref(),
            Some("0403:abcd")
        );
        assert_eq!(normalized_vid_pid("403:abcd"), None);
        assert_eq!(normalized_vid_pid("0403-abcd"), None);
    }

    #[test]
/// PURPOSE: Assert Gen5 forbids relay fields and Gen4 requires confirmed channel.
/// USE: regression for generation-specific UART prerequisites.
    fn gen5_rejects_relay_fields_and_gen4_requires_confirmation() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::uart_handlers", "gen5_rejects_relay_fields_and_gen4_requires_confirmation");
        assert_eq!(
            validate_context(&context(5), &request()).unwrap(),
            (5, None)
        );

        let mut gen5_with_relay = request();
        gen5_with_relay.relay_id = Some(Uuid::new_v4());
        assert!(validate_context(&context(5), &gen5_with_relay).is_err());

        let mut gen4 = context(4);
        let mut gen4_request = request();
        gen4_request.relay_id = Some(Uuid::new_v4());
        gen4_request.channel_id = Some(Uuid::new_v4());
        gen4.relay_serial = Some("RELAY-A".into());
        gen4.relay_channel_id = gen4_request.channel_id;
        gen4.channel_number = Some(2);
        assert!(validate_context(&gen4, &gen4_request).is_err());
        gen4.relay_confirmed = true;
        assert_eq!(
            validate_context(&gen4, &gen4_request).unwrap(),
            (4, Some(2))
        );
    }
}
