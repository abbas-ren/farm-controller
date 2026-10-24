//! DTOs for the stale device-action watchdog (jobs stuck in `running` / `waiting`).

use crate::socket::types::farm::ServerEvent;

/// Action still marked in-flight past the configured stale threshold.
#[derive(Debug, sea_orm::FromQueryResult)]
pub struct StaleAction {
    pub id: uuid::Uuid,
    pub job_type: String,
    #[sea_orm(alias = "testId")]
    pub test_id: Option<String>,
    #[sea_orm(alias = "releaseId")]
    pub release_id: Option<uuid::Uuid>,
    /// Projected from `targetDeviceId` via `column_as(..., "device_id")`.
    pub device_id: Option<String>,
    #[sea_orm(alias = "deviceType")]
    pub device_type: String,
    #[sea_orm(alias = "userId")]
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
