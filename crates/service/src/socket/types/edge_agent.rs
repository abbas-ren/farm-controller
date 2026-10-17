use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EdgeAgentMessagePayload {}
