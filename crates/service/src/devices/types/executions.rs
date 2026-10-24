//! Test execution create/update/list/report wire types.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Request/response body type `ExecutionListQuery` for device HTTP handlers.
#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionListQuery {
    /// `ExecutionListQuery.sort_by` used by device HTTP handlers.
    pub sort_by: Option<String>,
    /// `ExecutionListQuery.desc` used by device HTTP handlers.
    pub desc: Option<String>,
    /// Free-text filter applied by the handler/repository.
    pub search: Option<String>,
    /// 1-based page number for paginated lists.
    pub page: Option<i64>,
    /// Maximum rows to return (capped by MAX_PAGE_SIZE).
    pub limit: Option<i64>,
}

/// Request/response body type `ExecutionList` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionList {
    /// Payload body preserved for frontend compatibility.
    pub data: Vec<serde_json::Value>,
    /// Total matching rows before pagination.
    pub total: u64,
    /// `ExecutionList.current_page` used by device HTTP handlers.
    pub current_page: u64,
    /// `ExecutionList.total_pages` used by device HTTP handlers.
    pub total_pages: u64,
}

/// Request/response body type `ActiveExecutionRecord` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActiveExecutionRecord {
    /// Stable device identifier (often MAC-derived).
    pub device_id: Option<String>,
    /// Outcome or lifecycle status for this record.
    pub status: String,
    /// Test execution identifier.
    pub test_id: String,
}

/// Request/response body type `ExecutionCaseRecord` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCaseRecord {
    /// Persisted or catalog test-case identifier.
    pub test_case_id: i64,
    /// TestRail/QMetry suite identifier.
    pub suite_id: i64,
    /// `ExecutionCaseRecord.result` used by device HTTP handlers.
    pub result: Option<String>,
    /// `ExecutionCaseRecord.comment` used by device HTTP handlers.
    pub comment: Option<String>,
    /// `ExecutionCaseRecord.jira_defect` used by device HTTP handlers.
    pub jira_defect: Option<String>,
    /// `ExecutionCaseRecord.title` used by device HTTP handlers.
    pub title: String,
    /// `ExecutionCaseRecord.suite_name` used by device HTTP handlers.
    pub suite_name: String,
    /// `ExecutionCaseRecord.script_file` used by device HTTP handlers.
    pub script_file: Option<String>,
    /// `ExecutionCaseRecord.id` used by device HTTP handlers.
    pub id: i64,
    /// Last-update timestamp (RFC3339 or DB datetime).
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
    /// `ExecutionCaseRecord.output_file_path` used by device HTTP handlers.
    pub output_file_path: Option<String>,
    /// `ExecutionCaseRecord.dmesg_file_path` used by device HTTP handlers.
    pub dmesg_file_path: Option<String>,
}

/// Request/response body type `ExecutionReportRecord` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReportRecord {
    /// Resource identifier.
    pub id: Uuid,
    /// `ExecutionReportRecord.test_execution_id` used by device HTTP handlers.
    pub test_execution_id: String,
    /// Outcome or lifecycle status for this record.
    pub status: String,
    /// Device type string used for build/artifact matching.
    pub device_type: String,
    /// `ExecutionReportRecord.build_version` used by device HTTP handlers.
    pub build_version: String,
    /// `ExecutionReportRecord.created_by` used by device HTTP handlers.
    pub created_by: String,
    /// `ExecutionReportRecord.upload_error` used by device HTTP handlers.
    pub upload_error: Option<String>,
    /// `ExecutionReportRecord.created_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    /// `ExecutionReportRecord.updated_at` used by device HTTP handlers.
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

/// Request/response body type `BuildExecutionList` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BuildExecutionList {
    /// Maximum rows to return (capped by MAX_PAGE_SIZE).
    pub limit: i64,
    /// Byte offset for resumable uploads or pagination.
    pub offset: i64,
    /// Total matching rows before pagination.
    pub total: i64,
    /// `BuildExecutionList.executions` used by device HTTP handlers.
    pub executions: Vec<serde_json::Value>,
}

/// Request/response body type `ExecutionCaseInput` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCaseInput {
    /// Persisted or catalog test-case identifier.
    pub test_case_id: i64,
    /// Display name for the resource.
    pub name: Option<String>,
    /// `ExecutionCaseInput.execution_id` used by device HTTP handlers.
    pub execution_id: Option<String>,
    /// TestRail/QMetry suite identifier.
    pub suite_id: i64,
    /// `ExecutionCaseInput.script_file` used by device HTTP handlers.
    pub script_file: String,
    /// `ExecutionCaseInput.suite_name` used by device HTTP handlers.
    pub suite_name: String,
    /// `ExecutionCaseInput.title` used by device HTTP handlers.
    pub title: String,
    /// TestRail/QMetry plan identifier.
    pub plan_id: i64,
    /// Sort direction (asc/desc).
    pub order: Option<i64>,
    /// `ExecutionCaseInput.priority_id` used by device HTTP handlers.
    pub priority_id: Option<i64>,
    /// `ExecutionCaseInput.result` used by device HTTP handlers.
    pub result: Option<String>,
    /// `ExecutionCaseInput.pre_condition` used by device HTTP handlers.
    pub pre_condition: Option<String>,
    /// `ExecutionCaseInput.labels` used by device HTTP handlers.
    #[serde(default)]
    pub labels: Vec<String>,
}

/// Request/response body type `ExecutionSelectionExclusions` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSelectionExclusions {
    /// `ExecutionSelectionExclusions.suites` used by device HTTP handlers.
    #[serde(default)]
    pub suites: Vec<String>,
    /// `ExecutionSelectionExclusions.cases_by_suite` used by device HTTP handlers.
    #[serde(default, alias = "cases")]
    pub cases_by_suite: BTreeMap<String, Vec<String>>,
}

/// Request/response body type `ExecutionSuiteSelection` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSuiteSelection {
    /// TestRail/QMetry suite identifier.
    pub suite_id: String,
    /// `ExecutionSuiteSelection.select_all` used by device HTTP handlers.
    pub select_all: bool,
    /// `ExecutionSuiteSelection.cases` used by device HTTP handlers.
    pub cases: Option<Vec<String>>,
    /// `ExecutionSuiteSelection.exclude_cases` used by device HTTP handlers.
    pub exclude_cases: Option<Vec<String>>,
}

/// Enumerated wire value `ExecutionSelection` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(tag = "mode")]
pub enum ExecutionSelection {
    #[serde(rename = "ALL")]
    All {
        #[serde(rename = "planId")]
        plan_id: String,
        #[serde(rename = "planName")]
        plan_name: Option<String>,
        exclude: Option<ExecutionSelectionExclusions>,
    },
    #[serde(rename = "PARTIAL")]
    Partial {
        #[serde(rename = "planId")]
        plan_id: String,
        #[serde(rename = "planName")]
        plan_name: Option<String>,
        suites: Vec<ExecutionSuiteSelection>,
    },
}

/// Request/response body type `CreateExecutionRequest` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateExecutionRequest {
    /// Display name for the resource.
    pub name: Option<String>,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    pub device_family: String,
    /// `CreateExecutionRequest.test_plan_name` used by device HTTP handlers.
    pub test_plan_name: Option<String>,
    /// Device type string used for build/artifact matching.
    pub device_type: String,
    /// Stable device identifier (often MAC-derived).
    pub device_id: Option<String>,
    /// Firmware/build inventory identifier.
    pub build_id: String,
    /// `CreateExecutionRequest.test_queue_id` used by device HTTP handlers.
    pub test_queue_id: Option<String>,
    /// `CreateExecutionRequest.test_plan_id` used by device HTTP handlers.
    pub test_plan_id: Option<i64>,
    /// `CreateExecutionRequest.test_suites` used by device HTTP handlers.
    #[serde(default)]
    pub test_suites: Vec<i64>,
    /// `CreateExecutionRequest.is_all_selected` used by device HTTP handlers.
    #[serde(default)]
    pub is_all_selected: bool,
    /// `CreateExecutionRequest.logs` used by device HTTP handlers.
    pub logs: Option<Vec<String>>,
    /// `CreateExecutionRequest.created_by` used by device HTTP handlers.
    pub created_by: Option<String>,
    /// `CreateExecutionRequest.updated_by` used by device HTTP handlers.
    pub updated_by: Option<String>,
    /// `CreateExecutionRequest.test_cases` used by device HTTP handlers.
    #[serde(default)]
    pub test_cases: BTreeMap<i64, Vec<ExecutionCaseInput>>,
    /// `CreateExecutionRequest.selection` used by device HTTP handlers.
    pub selection: Option<ExecutionSelection>,
}

/// Request/response body type `UpdateExecutionRequest` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateExecutionRequest {
    /// Test execution identifier.
    #[serde(rename = "testID")]
    pub test_id: String,
    /// `UpdateExecutionRequest.comments` used by device HTTP handlers.
    #[serde(default)]
    pub comments: String,
    /// Firmware/build inventory identifier.
    pub build_id: Option<String>,
    /// `UpdateExecutionRequest.log` used by device HTTP handlers.
    #[serde(default)]
    pub log: bool,
    /// `UpdateExecutionRequest.result` used by device HTTP handlers.
    pub result: String,
}

/// Request/response body type `ExecutionStatusRequest` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionStatusRequest {
    /// Outcome or lifecycle status for this record.
    pub status: String,
}

/// Request/response body type `ExecutionStatusResponse` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionStatusResponse {
    /// Test execution identifier.
    pub test_id: String,
    /// Outcome or lifecycle status for this record.
    pub status: String,
}
