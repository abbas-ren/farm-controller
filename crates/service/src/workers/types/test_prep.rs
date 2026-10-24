//! Claim shapes for the test-preparation worker (legacy TestWorker / ArtifactsWorker prep).

/// Test execution ready to move from queue into artifact / script preparation.
#[derive(Debug, sea_orm::FromQueryResult)]
pub struct TestPreparation {
    pub test_id: String,
    pub device_type: String,
    pub build_id: String,
    pub build_version: String,
    pub test_plan_id: i64,
    pub test_plan_name: String,
    pub name: String,
    pub is_all_selected: bool,
    pub created_by: String,
    pub test_cycle_id: Option<String>,
    pub artifacts: serde_json::Value,
}

/// Selected test-case row used when materializing scripts for a preparation job.
#[derive(Debug, sea_orm::FromQueryResult)]
pub struct TestPreparationCase {
    pub test_case_id: i64,
    pub script_file: String,
}
