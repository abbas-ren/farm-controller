//! Shared JSON DTOs embedded in persistence columns.
//!
//! These types are not SeaORM entities; they describe the shape of JSON blobs
//! stored on rows such as releases (`artifacts`) and test-execution selection
//! payloads. They replace ad-hoc `JSON` typing from the Sequelize models with
//! typed serde structures used when assembling or reading those columns.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// QMetry/TestRail case selection payload stored on a test execution.
///
/// Callers persist this when a user launches a run so workers can expand the
/// chosen plan/suites into concrete cases without re-querying the UI state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionInput {
    /// Selection strategy (`all`, suite-scoped, etc.).
    pub mode: String,
    /// External plan identifier (TestRail suite / QMetry plan).
    pub plan_id: String,
    /// Human-readable plan name for logs and UI.
    pub plan_name: Option<String>,
    /// Optional exclusions applied after suite expansion.
    pub exclude: Option<SelectionExclude>,
    /// Explicit suite selections when not selecting the entire plan.
    pub suites: Option<Vec<SelectionSuite>>,
}

/// Suites/cases to drop from an otherwise inclusive selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionExclude {
    /// Suite IDs omitted entirely.
    pub suites: Option<Vec<String>>,
    /// Per-suite case ID lists to omit.
    pub cases_by_suite: Option<std::collections::HashMap<String, Vec<String>>>,
}

/// One suite contribution inside [`SelectionInput`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionSuite {
    /// External suite/section identifier.
    pub suite_id: String,
    /// When true, every case in the suite is included unless excluded.
    pub select_all: bool,
    /// Explicit case IDs when `select_all` is false.
    pub cases: Option<Vec<String>>,
}

/// Metadata for a build artifact discovered on disk or NFS.
///
/// Serialized into `releases.artifacts` so flash/test workers know which files
/// belong to a release without rescanning the folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactInfo {
    /// File basename as shown in the UI.
    pub name: String,
    /// Size in bytes at last scan.
    pub size: i64,
    /// Last modification time from the filesystem.
    pub mtime: chrono::DateTime<chrono::Utc>,
    /// Absolute or NFS-relative path used by flash workers.
    pub path: String,
    /// Optional content hash for integrity checks.
    pub sha256: Option<String>,
}

/// Untyped JSON object alias for columns that remain schema-flexible.
pub type JsonObject = Value;
