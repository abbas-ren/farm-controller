use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::devices::RelayControllerAction;

pub struct RelaySyncJob {
    pub actions: Vec<RelayControllerAction>,
    pub relay_id: Option<uuid::Uuid>,
}

#[derive(Debug, sea_orm::FromQueryResult)]
pub struct ExpiredRelayConfiguration {
    pub id: Uuid,
    pub device_mac: String,
    pub channel_number: i32,
    pub relay_id: Option<Uuid>,
    pub controller_id: Option<String>,
}

#[derive(Clone)]
pub struct RelaySyncService {
    pub sender: mpsc::Sender<RelaySyncJob>,
    pub receiver: Arc<Mutex<Option<mpsc::Receiver<RelaySyncJob>>>>,
}
