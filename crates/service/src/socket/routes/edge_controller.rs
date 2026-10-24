//! Edge-controller WebSocket room: DC heartbeats, flash progress, mappings.

use crate::{
    devices::ControllerHeartbeat,
    socket::{
        helpers::{controller_heartbeat_payload, normalize_controller_id},
        types::{
            farm::ServerEvent,
            frontend::{Action, AlertLevel, FrontendMessagePayload},
        },
    },
    state::AppState,
};
use axum::extract::ws::{Message, WebSocket};
use std::sync::Arc;

/// Controller session.
pub(crate) async fn controller_session(
    state: Arc<AppState>,
    controller_id: String,
    mut socket: WebSocket,
) {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::edge_controller", "controller_session");
    tracing::info!(controller_id, "device controller WebSocket connected");
    while let Some(message) = socket.recv().await {
        let bytes = match message {
            Ok(Message::Binary(bytes)) => bytes,
            Ok(Message::Text(text)) => text.as_bytes().to_vec().into(),
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Err(error) => {
                tracing::warn!(%error, controller_id, "device controller WebSocket read failed");
                break;
            }
        };
        if let Err(error) = process_heartbeat(&state, &controller_id, &bytes).await {
            tracing::warn!(%error, controller_id, "invalid device controller heartbeat");
        }
    }
    tracing::info!(controller_id, "device controller WebSocket disconnected");
}

/// Process heartbeat.
pub(crate) async fn process_heartbeat(
    state: &AppState,
    controller_id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::edge_controller", "process_heartbeat");
    let heartbeat: ControllerHeartbeat =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if heartbeat.message_type != "heartbeat" {
        return Ok(());
    }
    let normalized_expected = normalize_controller_id(controller_id);
    let normalized_cid = normalize_controller_id(&heartbeat.uid);
    if normalized_expected != normalized_cid {
        return Err("heartbeat uid does not match the WebSocket identity".to_owned());
    }
    let repository = state
        .device_repository
        .as_ref()
        .ok_or_else(|| "device persistence is unavailable".to_owned())?;
    let result = repository
        .record_controller_heartbeat(&heartbeat)
        .await
        .map_err(|error| error.to_string())?;

    if result.state_changed {
        state.event_publisher.publish(ServerEvent::frontend(
            FrontendMessagePayload::DeviceControllerChanged {
                action: Action::Updated,
                controller_id: normalized_cid.clone(),
            },
        ))?;
    }
    if let Some(alert) = result.alert {
        state.event_publisher.publish(ServerEvent::frontend(
            FrontendMessagePayload::AlertDataPayload {
                level: Some(AlertLevel::Info),
                subtype: None,
                message: alert,
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
            },
        ))?;
    }
    state
        .event_publisher
        .publish(ServerEvent::frontend(controller_heartbeat_payload(
            normalized_cid,
            &heartbeat,
        )))
}
