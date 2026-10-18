use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EdgeAgentMessagePayload {}
