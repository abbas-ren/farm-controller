//! DTOs for the stale device-action watchdog (jobs stuck in `running` / `waiting`).

use crate::socket::types::farm::ServerEvent;

/// Action still marked in-flight past the configured stale threshold.
#[derive(Debug, sea_orm::FromQueryResult)]
pub struct StaleAction {
    pub id: uuid::Uuid,
    pub job_type: String,
    pub test_id: Option<String>,
    pub release_id: Option<uuid::Uuid>,
    pub device_id: Option<String>,
    pub device_type: String,
    pub user_id: String,
    pub build_id: Option<String>,
    pub created_by: Option<String>,
    pub version: Option<String>,
}

/// Side effects produced when a stale action is cancelled / failed.
pub struct ActionTransition {
    /// Farm WebSocket events to publish after the DB transition commits.
    pub events: Vec<ServerEvent>,
}
