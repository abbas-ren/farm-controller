//! Public request, response, status, and persistence-row types for reports.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportRequest {
    pub test_id: String,
    pub build_version: String,
    pub device_type: String,
    pub previous_test_id: Option<String>,
    pub previous_build_version: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Queued,
    Processing,
    Consolidating,
    GeneratingSanity,
    GeneratingPerf,
    GeneratingGraphs,
    BuildingReport,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportJobSnapshot {
    pub job_id: Uuid,
    pub status: ReportStatus,
    pub test_id: String,
    pub build_version: String,
    pub device_type: String,
    pub error: Option<String>,
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) created_by: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportAcknowledgement {
    pub status: &'static str,
    pub job_id: Uuid,
    pub test_id: String,
    pub build_version: String,
    pub device_type: String,
    pub message: &'static str,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfluenceUploadRequest {
    pub test_id: String,
    pub build_version: String,
    pub device_type: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfluenceUploadResponse {
    pub status: &'static str,
    pub page_id: String,
    pub subpage_title: String,
    pub parent_page_id: String,
    pub uploaded_images: Vec<String>,
    pub failed_images: Vec<String>,
}

#[derive(Clone, Debug, FromRow)]
pub struct TestCaseRecord {
    pub test_case_id: i64,
    pub suite_name: String,
    pub result: Option<String>,
    pub comment: Option<String>,
    pub command: Option<String>,
    pub jira_defect: Option<String>,
    pub output_file_path: Option<String>,
    pub labels: Option<Vec<String>>,
}
