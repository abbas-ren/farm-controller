use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportCreateRequest {
    pub device_type: String,
    pub device_family: String,
    pub release_id: Uuid,
    pub description: String,
    pub test_execution_id: Option<String>,
    pub attach_logs: Option<bool>,
    #[schema(format = Binary)]
    pub image: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum FaultyReportStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportRecord {
    pub id: Uuid,
    pub release_id: Uuid,
    pub device_type: String,
    pub device_family: String,
    pub last_test_execution_id: Option<String>,
    pub description: String,
    pub file_path: Option<String>,
    pub logs_path: Option<String>,
    pub status: FaultyReportStatus,
    pub created_by: String,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportUser {
    pub id: String,
    pub user_name: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportDetail {
    pub report: FaultyReportRecord,
    pub build_version: String,
    pub user: FaultyReportUser,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FaultyReportStatusRequest {
    pub status: String,
}
