use axum::extract::ws::{Message, WebSocket};
use std::sync::Arc;
use tracing::{info, warn};

use crate::{
    socket::helpers::device_heartbeat_events, 
    state::AppState,
};

pub(crate) async fn device_session(state: Arc<AppState>, device_id: String, mut socket: WebSocket) {
    info!(device_id, "device WebSocket connected");
    while let Some(message) = socket.recv().await {
        let bytes = match message {
            Ok(Message::Binary(bytes)) => bytes,
            Ok(Message::Text(text)) => text.as_bytes().to_vec().into(),
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Err(error) => {
                warn!(%error, device_id, "device WebSocket read failed");
                break;
            }
        };
        if let Err(error) = process_device_heartbeat(&state, &device_id, &bytes).await {
            warn!(%error, device_id, "invalid device heartbeat");
        }
    }
    info!(device_id, "device WebSocket disconnected");
}

pub(crate) async fn process_device_heartbeat(
    state: &AppState,
    device_id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let heartbeat: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if heartbeat.get("type").and_then(serde_json::Value::as_str) != Some("heartbeat") {
        return Ok(());
    }
    let repository = state
        .device_repository
        .as_ref()
        .ok_or_else(|| "device persistence is unavailable".to_owned())?;
    let result = repository
        .record_device_heartbeat(device_id, &heartbeat)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "approved device was not found".to_owned())?;
    for cancellation in &result.deferred_cancellations {
        crate::devices::test_execution_handlers::complete_deferred_cancellation(
            state,
            cancellation,
        )
        .await;
    }
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    for event in device_heartbeat_events(device_id, result, &timestamp) {
        state.event_publisher.publish(event)?;
    }
    Ok(())
}

