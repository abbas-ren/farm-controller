//! Alert list and mark-read request/response types for notification routes.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Request/response body type `AdminAlertListQuery` for device HTTP handlers.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct AdminAlertListQuery {
    /// `AdminAlertListQuery.sort_by` used by device HTTP handlers.
    pub sort_by: Option<String>,
    /// `AdminAlertListQuery.desc` used by device HTTP handlers.
    pub desc: Option<String>,
    /// 1-based page number for paginated lists.
    pub page: Option<String>,
    /// Maximum rows to return (capped by MAX_PAGE_SIZE).
    pub limit: Option<String>,
    /// Free-text filter applied by the handler/repository.
    pub search: Option<String>,
    /// `AdminAlertListQuery.filter` used by device HTTP handlers.
    pub filter: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct AllAlertListQuery {
    /// 1-based page number for paginated lists.
    pub page: Option<String>,
    /// Maximum rows to return (capped by MAX_PAGE_SIZE).
    pub limit: Option<String>,
    /// `AllAlertListQuery.from` used by device HTTP handlers.
    pub from: Option<String>,
    /// `AllAlertListQuery.to` used by device HTTP handlers.
    pub to: Option<String>,
}

/// Request/response body type `AlertList` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AlertList {
    /// Payload body preserved for frontend compatibility.
    pub data: Vec<serde_json::Value>,
    /// `AlertList.total_data` used by device HTTP handlers.
    pub total_data: i64,
    /// `AlertList.total_pages` used by device HTTP handlers.
    pub total_pages: i64,
    /// `AlertList.current_page` used by device HTTP handlers.
    pub current_page: i64,
    /// `AlertList.total_unread_count` used by device HTTP handlers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_unread_count: Option<i64>,
}
