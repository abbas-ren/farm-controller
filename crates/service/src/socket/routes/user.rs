use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use reqwest::StatusCode;

use crate::{
    socket::types::{
        farm::{EventErrorResponse, ServerEvent, UserMessageRequest, UserMessageResponse},
        frontend::FrontendMessagePayload,
    },
    state::AppState,
};

#[utoipa::path(post, path = "/api/v1/device/ws/send/message", tag = "Events", summary = "Dispatch a public user event", description = "Publishes the legacy `user` Socket.IO event with `{message,timestamp}` to connected browser clients.", request_body = UserMessageRequest, responses((status = 200, description = "Event dispatched", body = UserMessageResponse), (status = 500, description = "Event dispatch failed", body = EventErrorResponse)))]
pub(crate) async fn send_user_message(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UserMessageRequest>,
) -> Response {
    let user_data_payload = FrontendMessagePayload::UserDataPayload {
        method: "send_user_message".to_owned(),
        message: request.data.unwrap_or(serde_json::Value::Null),
        timestamp: Some(chrono::Utc::now().to_rfc3339()),
    };

    match state
        .event_publisher
        .publish(ServerEvent::frontend(user_data_payload))
    {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(error) => {
            tracing::error!(%error, "failed to dispatch user event");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"message": error})),
            )
                .into_response()
        }
    }
}
