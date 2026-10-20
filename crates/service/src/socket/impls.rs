use serde::Serialize;
use socketioxide::SocketIo;
use std::sync::OnceLock;
use tracing::info;

use crate::socket::types::{
    edge_agent::EdgeAgentMessagePayload,
    edge_controller::EdgeControllerMessagePayload,
    farm::{EventHub, ServerEvent},
    frontend::FrontendMessagePayload,
};

impl ServerEvent {
    pub(crate) fn frontend(payload: FrontendMessagePayload) -> Self {
        let payload =
            serde_json::to_value(&payload).expect("frontend payload must be serializable");
        let event_name = payload
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .expect("event field must be a string");
        Self {
            event: event_name.to_owned(),
            payload,
        }
    }

    #[allow(unused)]
    pub(crate) fn edge_agent(payload: EdgeAgentMessagePayload) -> Self {
        Self {
            event: "edge_agent".to_owned(),
            payload: serde_json::to_value(payload)
                .expect("edge agent payload must be serializable"),
        }
    }

    #[allow(unused)]
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
                if let Err(error) = socket.emit(&event.event, &event.payload) {
                    tracing::debug!(%error, event = event.event, "Socket.IO client emit failed");
                }
            }
        }

        let _ = self.sender.send(event);
        info!("Published event: {}", event_name);
        Ok(())
    }
}
