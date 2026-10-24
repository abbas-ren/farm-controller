//! Relay sync job channel and expired pending-config claim rows.
//!
//! Used by `relay_service` (legacy `RelayService` + `RelayConfigTimeoutService`).

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::devices::RelayControllerAction;

/// Batch of controller actions to push for one relay (or global sync).
pub struct RelaySyncJob {
    pub actions: Vec<RelayControllerAction>,
    pub relay_id: Option<uuid::Uuid>,
}

/// Pending relay configuration past `device_controller_config_timeout`.
#[derive(Debug, sea_orm::FromQueryResult)]
pub struct ExpiredRelayConfiguration {
    pub id: Uuid,
    pub device_mac: String,
    pub channel_number: i32,
    pub relay_id: Option<Uuid>,
    pub controller_id: Option<String>,
}

/// Shared mpsc handle so HTTP handlers enqueue sync work and the worker drains it.
#[derive(Clone)]
pub struct RelaySyncService {
    pub sender: mpsc::Sender<RelaySyncJob>,
    /// Taken once by the worker; `None` after start.
    pub receiver: Arc<Mutex<Option<mpsc::Receiver<RelaySyncJob>>>>,
}
