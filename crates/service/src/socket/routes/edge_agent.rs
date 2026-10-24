//! Edge-agent WebSocket room: agent registration and command fan-out.

use axum::extract::ws::{Message, WebSocket};
use std::sync::Arc;
use tracing::{info, warn};

use crate::{socket::helpers::device_heartbeat_events, state::AppState};

/// Device session.
pub(crate) async fn device_session(state: Arc<AppState>, device_id: String, mut socket: WebSocket) {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::edge_agent", "device_session");
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

/// Process device heartbeat.
pub(crate) async fn process_device_heartbeat(
    state: &AppState,
    device_id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::edge_agent", "process_device_heartbeat");
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
    for log in &result.deferred_test_logs {
        if let Err(error) =
            crate::devices::test_log::append_test_log(state, &log.test_id, &log.message).await
        {
            warn!(%error, test_id = %log.test_id, "failed to append deferred heartbeat test log");
        }
    }
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let should_wake = !result.post_update_events.is_empty();
    for event in device_heartbeat_events(device_id, result, &timestamp) {
        state.event_publisher.publish(event)?;
    }
    // Post-IPL requeue / completion side effects need immediate device_action claim.
    if should_wake {
        state.wake_workers();
    }
    Ok(())
}
