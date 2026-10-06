use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

fn default_log_type() -> String {
    "general".to_owned()
}

fn default_log_level() -> String {
    "info".to_owned()
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogCreateRequest {
    #[serde(rename = "type", default = "default_log_type")]
    pub log_type: String,
    pub reference_id: String,
    pub data: serde_json::Value,
    #[serde(default = "default_log_level")]
    pub level: String,
    #[schema(value_type = String, format = DateTime)]
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogListQuery {
    #[serde(rename = "type")]
    pub log_type: Option<String>,
    pub reference_id: Option<String>,
    pub level: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub query: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: i64,
    #[serde(rename = "type")]
    pub log_type: String,
    pub reference_id: String,
    pub data: serde_json::Value,
    pub level: String,
    #[schema(value_type = String, format = DateTime)]
    pub timestamp: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogList {
    pub logs: Vec<LogEntry>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}
