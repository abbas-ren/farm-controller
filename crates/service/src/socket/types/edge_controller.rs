//! Messages exchanged with edge-controller WebSocket peers.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EdgeControllerMessagePayload {}
