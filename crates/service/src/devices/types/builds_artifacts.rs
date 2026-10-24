//! Build inventory, upload, flag, and artifact configuration wire types.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Request/response body type `BuildsQuery` for device HTTP handlers.
#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildsQuery {
    /// Device type string used for build/artifact matching.
    pub device_type: Option<String>,
}

/// Request/response body type `BuildListQuery` for device HTTP handlers.
#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildListQuery {
    /// Free-text filter applied by the handler/repository.
    pub search: Option<String>,
    /// Device type string used for build/artifact matching.
    pub device_type: Option<String>,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    pub device_family: Option<String>,
    /// Whether an admin flagged this build.
    pub flagged: Option<String>,
    /// `BuildListQuery.build_version` used by device HTTP handlers.
    pub build_version: Option<String>,
    /// 1-based page number for paginated lists.
    pub page: Option<i64>,
    /// Maximum rows to return (capped by MAX_PAGE_SIZE).
    pub limit: Option<i64>,
    /// `BuildListQuery.sort_by` used by device HTTP handlers.
    pub sort_by: Option<String>,
    /// `BuildListQuery.sort_order` used by device HTTP handlers.
    pub sort_order: Option<String>,
}

/// Request/response body type `BuildList` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildList {
    /// `BuildList.builds` used by device HTTP handlers.
    pub builds: Vec<serde_json::Value>,
    /// `BuildList.total_count` used by device HTTP handlers.
    pub total_count: u64,
    /// `BuildList.current_page` used by device HTTP handlers.
    pub current_page: u64,
    /// `BuildList.total_pages` used by device HTTP handlers.
    pub total_pages: u64,
    /// `BuildList.requested_count` used by device HTTP handlers.
    pub requested_count: u64,
}

/// Request/response body type `BuildFilters` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildFilters {
    /// `BuildFilters.device_types` used by device HTTP handlers.
    pub device_types: Vec<String>,
    /// `BuildFilters.device_families` used by device HTTP handlers.
    pub device_families: Vec<String>,
}

/// Request/response body type `BuildUploadInitRequest` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildUploadInitRequest {
    /// `BuildUploadInitRequest.file_count` used by device HTTP handlers.
    #[schema(minimum = 1, maximum = 5, example = 1)]
    pub file_count: i32,
}

/// Request/response body type `BuildUploadInitResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildUploadInitResponse {
    /// TUS or init-upload identifier.
    #[schema(example = "8e8f632f-18ee-469f-a472-79245f37c842")]
    pub upload_id: Uuid,
}

/// Request/response body type `BuildUploadResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildUploadResponse {
    /// Human-readable status or error text for the client.
    #[schema(example = "Upload successful")]
    pub message: String,
    /// `BuildUploadResponse.upload_id` used by device HTTP handlers.
    #[schema(example = "8e8f632f-18ee-469f-a472-79245f37c842")]
    pub upload_id: String,
    /// `BuildUploadResponse.filename` used by device HTTP handlers.
    #[schema(example = "RZG2L__v1.2.3.zip")]
    pub filename: String,
}

/// Request/response body type `BuildFlagResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildFlagResponse {
    /// Resource identifier.
    pub id: String,
    /// `BuildFlagResponse.is_faulty` used by device HTTP handlers.
    pub is_faulty: bool,
}

/// Request/response body type `BuildFlagRequest` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildFlagRequest {
    /// `BuildFlagRequest.is_faulty` used by device HTTP handlers.
    pub is_faulty: bool,
}

/// Request/response body type `BuildStatusRequest` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildStatusRequest {
    /// Outcome or lifecycle status for this record.
    pub status: String,
}

/// Request/response body type `BuildStatusResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildStatusResponse {
    /// Resource identifier.
    pub id: String,
    /// Outcome or lifecycle status for this record.
    pub status: String,
    /// `BuildStatusResponse.is_faulty` used by device HTTP handlers.
    pub is_faulty: bool,
}

/// Request/response body type `DeviceTypeFolder` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTypeFolder {
    /// Device type string used for build/artifact matching.
    pub device_type: String,
    /// `DeviceTypeFolder.folder_name` used by device HTTP handlers.
    pub folder_name: String,
    /// Hardware family label (Gen3/Gen4/Gen5, etc.).
    pub device_family: String,
    /// `DeviceTypeFolder.default_version` used by device HTTP handlers.
    pub default_version: String,
}

/// Enumerated wire value `DeviceTypeFolderRequest` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum DeviceTypeFolderRequest {
    One(DeviceTypeFolder),
    Many(Vec<DeviceTypeFolder>),
}

/// Request/response body type `DefaultArtifactCopyQuery` for device HTTP handlers.
#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
pub struct DefaultArtifactCopyQuery {
    /// When true, skip EdgeAgent acknowledgement and delete anyway.
    #[serde(default)]
    pub force: bool,
}

/// Request/response body type `DefaultArtifactCopyResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct DefaultArtifactCopyResponse {
    /// Whether the legacy API considers the operation successful.
    pub success: bool,
    /// Human-readable status or error text for the client.
    pub message: &'static str,
}
