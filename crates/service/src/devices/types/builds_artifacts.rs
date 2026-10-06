use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildsQuery {
    pub device_type: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildListQuery {
    pub search: Option<String>,
    pub device_type: Option<String>,
    pub device_family: Option<String>,
    pub flagged: Option<String>,
    pub build_version: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildList {
    pub builds: Vec<serde_json::Value>,
    pub total_count: u64,
    pub current_page: u64,
    pub total_pages: u64,
    pub requested_count: u64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildFilters {
    pub device_types: Vec<String>,
    pub device_families: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildUploadInitRequest {
    #[schema(minimum = 1, maximum = 5, example = 1)]
    pub file_count: i32,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildUploadInitResponse {
    #[schema(example = "8e8f632f-18ee-469f-a472-79245f37c842")]
    pub upload_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildUploadResponse {
    #[schema(example = "Upload successful")]
    pub message: String,
    #[schema(example = "8e8f632f-18ee-469f-a472-79245f37c842")]
    pub upload_id: String,
    #[schema(example = "RZG2L__v1.2.3.zip")]
    pub filename: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildFlagResponse {
    pub id: String,
    pub is_faulty: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildFlagRequest {
    pub is_faulty: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTypeFolder {
    pub device_type: String,
    pub folder_name: String,
    pub device_family: String,
    pub default_version: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum DeviceTypeFolderRequest {
    One(DeviceTypeFolder),
    Many(Vec<DeviceTypeFolder>),
}

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
pub struct DefaultArtifactCopyQuery {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DefaultArtifactCopyResponse {
    pub success: bool,
    pub message: &'static str,
}
