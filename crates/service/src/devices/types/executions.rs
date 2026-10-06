use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionListQuery {
    pub sort_by: Option<String>,
    pub desc: Option<String>,
    pub search: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionList {
    pub data: Vec<serde_json::Value>,
    pub total: u64,
    pub current_page: u64,
    pub total_pages: u64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActiveExecutionRecord {
    pub device_id: Option<String>,
    pub status: String,
    pub test_id: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCaseRecord {
    pub test_case_id: i64,
    pub suite_id: i64,
    pub result: Option<String>,
    pub comment: Option<String>,
    pub jira_defect: Option<String>,
    pub title: String,
    pub suite_name: String,
    pub script_file: Option<String>,
    pub id: i64,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
    pub output_file_path: Option<String>,
    pub dmesg_file_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReportRecord {
    pub id: Uuid,
    pub test_execution_id: String,
    pub status: String,
    pub device_type: String,
    pub build_version: String,
    pub created_by: String,
    pub upload_error: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BuildExecutionList {
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
    pub executions: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCaseInput {
    pub test_case_id: i64,
    pub name: Option<String>,
    pub execution_id: Option<String>,
    pub suite_id: i64,
    pub script_file: String,
    pub suite_name: String,
    pub title: String,
    pub plan_id: i64,
    pub order: Option<i64>,
    pub priority_id: Option<i64>,
    pub result: Option<String>,
    pub pre_condition: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSelectionExclusions {
    #[serde(default)]
    pub suites: Vec<String>,
    #[serde(default, alias = "cases")]
    pub cases_by_suite: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSuiteSelection {
    pub suite_id: String,
    pub select_all: bool,
    pub cases: Option<Vec<String>>,
    pub exclude_cases: Option<Vec<String>>,
}

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

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateExecutionRequest {
    pub name: Option<String>,
    pub device_family: String,
    pub test_plan_name: Option<String>,
    pub device_type: String,
    pub device_id: Option<String>,
    pub build_id: String,
    pub test_queue_id: Option<String>,
    pub test_plan_id: Option<i64>,
    #[serde(default)]
    pub test_suites: Vec<i64>,
    #[serde(default)]
    pub is_all_selected: bool,
    pub logs: Option<Vec<String>>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    #[serde(default)]
    pub test_cases: BTreeMap<i64, Vec<ExecutionCaseInput>>,
    pub selection: Option<ExecutionSelection>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateExecutionRequest {
    #[serde(rename = "testID")]
    pub test_id: String,
    #[serde(default)]
    pub comments: String,
    pub build_id: Option<String>,
    #[serde(default)]
    pub log: bool,
    pub result: String,
}
