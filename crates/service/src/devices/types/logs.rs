//! Farm log create/list/search wire types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// PURPOSE: Serde default log type when omitted from create-log body.
/// USE: Deserialization of log create requests.
fn default_log_type() -> String {
    let _api_trace = crate::observability::ApiTrace::enter("devices::types::logs", "default_log_type");
    "general".to_owned()
}

/// PURPOSE: Serde default log level when omitted from create-log body.
/// USE: Deserialization of log create requests.
fn default_log_level() -> String {
    let _api_trace = crate::observability::ApiTrace::enter("devices::types::logs", "default_log_level");
    "info".to_owned()
}

/// Request/response body type `LogCreateRequest` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogCreateRequest {
    /// `LogCreateRequest.log_type` used by device HTTP handlers.
    #[serde(rename = "type", default = "default_log_type")]
    pub log_type: String,
    /// `LogCreateRequest.reference_id` used by device HTTP handlers.
    pub reference_id: String,
    /// Payload body preserved for frontend compatibility.
    pub data: serde_json::Value,
    /// Alert severity level.
    #[serde(default = "default_log_level")]
    pub level: String,
    /// RFC3339 timestamp for the event or record.
    #[schema(value_type = String, format = DateTime)]
    pub timestamp: Option<DateTime<Utc>>,
}

/// Request/response body type `LogListQuery` for device HTTP handlers.
#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogListQuery {
    /// `LogListQuery.log_type` used by device HTTP handlers.
    #[serde(rename = "type")]
    pub log_type: Option<String>,
    /// `LogListQuery.reference_id` used by device HTTP handlers.
    pub reference_id: Option<String>,
    /// Severity or log level.
    pub level: Option<String>,
    /// 1-based page number for paginated lists.
    pub page: Option<i64>,
    /// `LogListQuery.page_size` used by device HTTP handlers.
    pub page_size: Option<i64>,
    /// Allowlisted sort column name.
    pub sort: Option<String>,
    /// Sort direction (asc/desc).
    pub order: Option<String>,
    /// `LogListQuery.query` used by device HTTP handlers.
    pub query: Option<String>,
}

/// Request/response body type `LogEntry` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Resource identifier.
    pub id: i64,
    /// `LogEntry.log_type` used by device HTTP handlers.
    #[serde(rename = "type")]
    pub log_type: String,
    /// `LogEntry.reference_id` used by device HTTP handlers.
    pub reference_id: String,
    /// Payload body preserved for frontend compatibility.
    pub data: serde_json::Value,
    /// Severity or log level.
    pub level: String,
    /// RFC3339 timestamp for the event or record.
    #[schema(value_type = String, format = DateTime)]
    pub timestamp: DateTime<Utc>,
    /// `LogEntry.created_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    /// `LogEntry.updated_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

/// Request/response body type `LogList` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogList {
    /// `LogList.logs` used by device HTTP handlers.
    pub logs: Vec<LogEntry>,
    /// Total matching rows before pagination.
    pub total: i64,
    /// 1-based page number for paginated lists.
    pub page: i64,
    /// `LogList.page_size` used by device HTTP handlers.
    pub page_size: i64,
}
