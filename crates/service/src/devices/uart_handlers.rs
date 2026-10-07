use std::sync::Arc;

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{auth, error::ErrorResponse, state::AppState};

use super::{
    EDGE_CONTROLLER_PORT, UART_CONFIGURATION_TIMEOUT, UartConfigurationRequest,
    UartConfigurationResponse, error_response,
};

#[derive(FromRow)]
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

fn normalized_vid_pid(value: &str) -> Option<String> {
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

async fn load_context(
    state: &AppState,
    request: &UartConfigurationRequest,
) -> Result<UartContext, sqlx::Error> {
    let database = state
        .database
        .as_ref()
        .expect("database checked by handler");
    sqlx::query_as::<_, UartContext>(
        r#"SELECT d."deviceId" AS device_id,
                  COALESCE(NULLIF(d."macAddress", ''), d."deviceId") AS device_mac,
                  CASE
                                        WHEN replace(lower(concat_ws(' ', d."deviceFamily", d."deviceType", dc."deviceFamily")), ' ', '') LIKE '%gen3%' THEN 3
                                        WHEN replace(lower(concat_ws(' ', d."deviceFamily", d."deviceType", dc."deviceFamily")), ' ', '') LIKE '%gen4%' THEN 4
                                        WHEN replace(lower(concat_ws(' ', d."deviceFamily", d."deviceType", dc."deviceFamily")), ' ', '') LIKE '%gen5%' THEN 5
                    ELSE 0
                  END::smallint AS generation,
                  dc."ipAddress" AS controller_address,
                  r."serialNumber" AS relay_serial,
                  rc."channelNumber" AS channel_number,
                  rc.id AS relay_channel_id,
                  COALESCE((
                    SELECT pending.status = 'confirmed'
                    FROM pending_relay_configs pending
                    WHERE pending."relayChannelId" = rc.id
                      AND regexp_replace(lower(pending."deviceMac"), '[:-]', '', 'g') =
                          regexp_replace(lower(COALESCE(NULLIF(d."macAddress", ''), d."deviceId")), '[:-]', '', 'g')
                    ORDER BY pending."createdAt" DESC
                    LIMIT 1
                  ), false) AS relay_confirmed
           FROM devices d
           JOIN device_controllers dc
             ON dc."deviceControllerId" = $2 AND dc."deletedAt" IS NULL
           LEFT JOIN relays r
             ON r.id = $3 AND r."deviceControllerId" = dc."deviceControllerId"
            AND r."deletedAt" IS NULL
           LEFT JOIN relay_channels rc
             ON rc.id = $4 AND rc."relayId" = r.id AND rc."deviceId" = d."deviceId"
            AND rc."deletedAt" IS NULL
           WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL"#,
    )
    .bind(request.device_id.trim())
    .bind(request.controller_id.trim())
    .bind(request.relay_id)
    .bind(request.channel_id)
    .fetch_one(database.pool())
    .await
}

fn validate_context(
    context: &UartContext,
    request: &UartConfigurationRequest,
) -> Result<(u8, Option<u8>), &'static str> {
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
pub(crate) async fn configure_uart(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<UartConfigurationRequest>, JsonRejection>,
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
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    }
    let context = match load_context(&state, &request).await {
        Ok(context) => context,
        Err(sqlx::Error::RowNotFound) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Device or controller was not found",
            );
        }
        Err(error) => {
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
    let mut transaction = match database.pool().begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "persistence",
                &error.to_string(),
            );
        }
    };
    if let Err(error) = sqlx::query(
        r#"INSERT INTO device_uart_mappings
             (id, "deviceId", "controllerId", "relayChannelId", "deviceMac", generation,
              "requestedVidPid", tty, "usbSerial", interface, topology, connection, verified,
              "verifiedAt", "createdAt", "updatedAt")
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,true,now(),now(),now())
           ON CONFLICT ("deviceId") DO UPDATE SET
             "controllerId"=EXCLUDED."controllerId", "relayChannelId"=EXCLUDED."relayChannelId",
             "deviceMac"=EXCLUDED."deviceMac", generation=EXCLUDED.generation,
             "requestedVidPid"=EXCLUDED."requestedVidPid", tty=EXCLUDED.tty,
             "usbSerial"=EXCLUDED."usbSerial", interface=EXCLUDED.interface,
             topology=EXCLUDED.topology, connection=EXCLUDED.connection, verified=true,
             "verifiedAt"=now(), "updatedAt"=now()"#,
    )
    .bind(Uuid::new_v4())
    .bind(&context.device_id)
    .bind(request.controller_id.trim())
    .bind(context.relay_channel_id)
    .bind(&context.device_mac)
    .bind(i16::from(generation))
    .bind(&verified.vid_pid)
    .bind(&verified.tty)
    .bind(&verified.usb_serial)
    .bind(i16::from(verified.interface))
    .bind(&verified.topology)
    .bind(&verified.connection)
    .execute(&mut *transaction)
    .await
    {
        tracing::error!(%error, "failed to persist UART mapping");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "persistence",
            "Failed to persist verified UART mapping",
        );
    }
    let normalized_mac = context
        .device_mac
        .to_ascii_lowercase()
        .replace([':', '-'], "");
    let mapping = serde_json::json!({
        normalized_mac: {
            "uart": verified.tty,
            "uartVidPid": verified.vid_pid,
            "uartTopology": verified.topology,
            "uartConnection": verified.connection,
            "generation": generation
        }
    });
    if let Err(error) = sqlx::query(
        r#"UPDATE device_controllers SET mappings = COALESCE(mappings, '{}'::jsonb) || $2::jsonb,
             "updatedAt" = now() WHERE "deviceControllerId" = $1"#,
    )
    .bind(request.controller_id.trim())
    .bind(sqlx::types::Json(mapping))
    .execute(&mut *transaction)
    .await
    {
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

    fn request() -> UartConfigurationRequest {
        UartConfigurationRequest {
            controller_id: "controller-1".into(),
            device_id: "device-1".into(),
            relay_id: None,
            channel_id: None,
            uart_vid_pid: "0403:6010".into(),
        }
    }

    #[test]
    fn vid_pid_is_normalized_and_strictly_validated() {
        assert_eq!(
            normalized_vid_pid(" 0403:AbCd ").as_deref(),
            Some("0403:abcd")
        );
        assert_eq!(normalized_vid_pid("403:abcd"), None);
        assert_eq!(normalized_vid_pid("0403-abcd"), None);
    }

    #[test]
    fn gen5_rejects_relay_fields_and_gen4_requires_confirmation() {
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
