use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionInput {
    pub mode: String,
    pub plan_id: String,
    pub plan_name: Option<String>,
    pub exclude: Option<SelectionExclude>,
    pub suites: Option<Vec<SelectionSuite>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionExclude {
    pub suites: Option<Vec<String>>,
    pub cases_by_suite: Option<std::collections::HashMap<String, Vec<String>>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionSuite {
    pub suite_id: String,
    pub select_all: bool,
    pub cases: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactInfo {
    pub name: String,
    pub size: i64,
    pub mtime: chrono::DateTime<chrono::Utc>,
    pub path: String,
    pub sha256: Option<String>,
}

pub type JsonObject = Value;
