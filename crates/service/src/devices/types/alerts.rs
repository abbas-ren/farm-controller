use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct AdminAlertListQuery {
    pub sort_by: Option<String>,
    pub desc: Option<String>,
    pub page: Option<String>,
    pub limit: Option<String>,
    pub search: Option<String>,
    pub filter: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct AllAlertListQuery {
    pub page: Option<String>,
    pub limit: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AlertList {
    pub data: Vec<serde_json::Value>,
    pub total_data: i64,
    pub total_pages: i64,
    pub current_page: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_unread_count: Option<i64>,
}
