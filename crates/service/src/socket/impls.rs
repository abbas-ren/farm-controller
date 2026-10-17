use std::sync::{Arc, OnceLock};

use axum::{
    Router,
    http::{HeaderMap, HeaderValue, header},
};
use socketioxide::{SocketIo, layer::SocketIoLayer};
use tracing::info;

use crate::{
    devices::ControllerHeartbeat,
    socket::{
        constants::BROWSER_BEARER_PREFIX,
        types::{
            edge_agent::EdgeAgentMessagePayload,
            edge_controller::EdgeControllerMessagePayload,
            farm::{EventHub, NativeSocketQuery, ServerEvent},
            frontend::FrontendMessagePayload,
        },
    },
    state::AppState,
};

impl ServerEvent {
    pub(crate) fn frontend(payload: FrontendMessagePayload) -> Self {
        Self {
            event: "frontend".to_owned(),
            payload: serde_json::to_value(payload).expect("general payload must be serializable"),
        }
    }

    pub(crate) fn edge_agent(payload: EdgeAgentMessagePayload) -> Self {
        Self {
            event: "edge_agent".to_owned(),
            payload: serde_json::to_value(payload)
                .expect("edge agent payload must be serializable"),
        }
    }

    pub(crate) fn edge_controller(payload: EdgeControllerMessagePayload) -> Self {
        Self {
            event: "edge_controller".to_owned(),
            payload: serde_json::to_value(payload)
                .expect("edge controller payload must be serializable"),
        }
    }
}

impl Default for EventHub {
    fn default() -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(256);
        Self {
            sender,
            socket_io: OnceLock::new(),
        }
    }
}

impl EventHub {
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ServerEvent> {
        self.sender.subscribe()
    }

    pub(crate) fn install_socket_io(&self, socket_io: SocketIo) {
        let _ = self.socket_io.set(socket_io);
    }

    pub async fn shutdown(&self) {
        if let Some(socket_io) = self.socket_io.get() {
            socket_io.close().await;
        }
    }

    pub fn publish(&self, event: ServerEvent) -> Result<(), String> {
        let event_name = event.event.clone();
        info!("Publishing event: {}", event_name);
        if let Some(socket_io) = self.socket_io.get() {
            for socket in socket_io.sockets() {
                if let Err(error) = socket.emit(&event.event, &event) {
                    tracing::debug!(%error, event = event.event, "Socket.IO client emit failed");
                }
            }
        }

        let _ = self.sender.send(event);
        info!("Published event: {}", event_name);
        Ok(())
    }
}
