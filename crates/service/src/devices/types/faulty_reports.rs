//! Faulty-report create/list/status wire types.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Request/response body type `FaultyReportCreateRequest` for device HTTP handlers.
#[derive(Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportCreateRequest {
    /// Device type string used for build/artifact matching.
    pub device_type: String,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    pub device_family: String,
    /// `FaultyReportCreateRequest.release_id` used by device HTTP handlers.
    pub release_id: Uuid,
    /// Optional human-readable description.
    pub description: String,
    /// `FaultyReportCreateRequest.test_execution_id` used by device HTTP handlers.
    pub test_execution_id: Option<String>,
    /// `FaultyReportCreateRequest.attach_logs` used by device HTTP handlers.
    pub attach_logs: Option<bool>,
    /// `FaultyReportCreateRequest.image` used by device HTTP handlers.
    #[schema(format = Binary)]
    pub image: Option<String>,
}

/// Enumerated wire value `FaultyReportStatus` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum FaultyReportStatus {
    Pending,
    Approved,
    Rejected,
}

/// Request/response body type `FaultyReportRecord` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportRecord {
    /// Resource identifier.
    pub id: Uuid,
    /// `FaultyReportRecord.release_id` used by device HTTP handlers.
    pub release_id: Uuid,
    /// Device type string used for build/artifact matching.
    pub device_type: String,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    pub device_family: String,
    /// `FaultyReportRecord.last_test_execution_id` used by device HTTP handlers.
    pub last_test_execution_id: Option<String>,
    /// `FaultyReportRecord.description` used by device HTTP handlers.
    pub description: String,
    /// `FaultyReportRecord.file_path` used by device HTTP handlers.
    pub file_path: Option<String>,
    /// `FaultyReportRecord.logs_path` used by device HTTP handlers.
    pub logs_path: Option<String>,
    /// Outcome or lifecycle status for this record.
    pub status: FaultyReportStatus,
    /// `FaultyReportRecord.created_by` used by device HTTP handlers.
    pub created_by: String,
    /// `FaultyReportRecord.created_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// `FaultyReportRecord.updated_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Request/response body type `FaultyReportUser` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportUser {
    /// Resource identifier.
    pub id: String,
    /// `FaultyReportUser.user_name` used by device HTTP handlers.
    pub user_name: String,
    /// `FaultyReportUser.first_name` used by device HTTP handlers.
    pub first_name: Option<String>,
    /// `FaultyReportUser.last_name` used by device HTTP handlers.
    pub last_name: Option<String>,
}

/// Request/response body type `FaultyReportDetail` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FaultyReportDetail {
    /// `FaultyReportDetail.report` used by device HTTP handlers.
    pub report: FaultyReportRecord,
    /// `FaultyReportDetail.build_version` used by device HTTP handlers.
    pub build_version: String,
    /// `FaultyReportDetail.user` used by device HTTP handlers.
    pub user: FaultyReportUser,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FaultyReportStatusRequest {
    /// Outcome or lifecycle status for this record.
    pub status: String,
}
