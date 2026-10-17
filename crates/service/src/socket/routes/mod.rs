pub mod edge_agent;
pub mod edge_controller;
pub mod terminal;
pub mod test_result;
pub mod upgrade;
pub mod user;
pub mod web_cli;

use crate::state::AppState;

use self::{upgrade::upgrade, user::send_user_message};

use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ws", get(upgrade))
        .route("/api/v1/device/ws/send/message", post(send_user_message))
}
