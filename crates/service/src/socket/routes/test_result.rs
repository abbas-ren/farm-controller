use crate::state::AppState;
use axum::extract::ws::close_code;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use std::sync::Arc;

pub(crate) async fn test_result_session(
    state: Arc<AppState>,
    test_id: String,
    device_id: String,
    mut socket: WebSocket,
) {
    tracing::info!(%test_id, %device_id, "test result WebSocket connected");
    while let Some(message) = socket.recv().await {
        let case_id = match message {
            Ok(Message::Text(text)) => text.trim().parse::<i64>().ok(),
            Ok(Message::Binary(bytes)) => std::str::from_utf8(&bytes)
                .ok()
                .and_then(|text| text.trim().parse::<i64>().ok()),
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(bytes)) => {
                if socket.send(Message::Pong(bytes)).await.is_err() {
                    break;
                }
                continue;
            }
            Ok(Message::Pong(_)) => continue,
            Err(error) => {
                tracing::warn!(%error, %test_id, %device_id, "test result WebSocket read failed");
                break;
            }
        };
        let Some(case_id) = case_id.filter(|case_id| *case_id > 0) else {
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: close_code::POLICY,
                    reason: "test case id must be a positive integer".into(),
                })))
                .await;
            break;
        };
        match crate::test_results::process_case(&state, &test_id, &device_id, case_id).await {
            Ok(result) => crate::test_results::sync_catalog(&state, &result).await,
            Err(error) => {
                tracing::warn!(%error, %test_id, %device_id, case_id, "failed to process native test result");
                let _ = socket
                    .send(Message::Close(Some(CloseFrame {
                        code: close_code::ERROR,
                        reason: "failed to process test result".into(),
                    })))
                    .await;
                break;
            }
        }
    }
    tracing::info!(%test_id, %device_id, "test result WebSocket disconnected");
}
