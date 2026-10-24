//! Software release / build catalog (`releases` table).
//!
//! Ports `Release.ts` / build & artifacts workers. Each row is a scanned or
//! uploaded build for a device type/family, including artifact JSON used by
//! flash workers.

use super::common::ArtifactInfo;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Whether the build came from an official pipeline or a custom upload.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_releases_buildType"
)]
pub enum BuildType {
    #[sea_orm(string_value = "custom")]
    Custom,
    #[sea_orm(string_value = "official")]
    Official,
}

/// Aggregate test outcome for the release across the farm.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_releases_status"
)]
pub enum ReleaseTestStatus {
    #[sea_orm(string_value = "untested")]
    Untested,
    #[sea_orm(string_value = "testing")]
    Testing,
    #[sea_orm(string_value = "passed")]
    Passed,
    #[sea_orm(string_value = "failed")]
    Failed,
}

/// One versioned build available for flashing and testing.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "releases", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub folder_name: String,
    pub version: String,
    /// JSON array of [`ArtifactInfo`] objects from the last scan.
    pub artifacts: Json,
    pub last_scanned_at: Option<DateTimeUtc>,
    pub tested_device_id: Option<String>,
    pub build_type: BuildType,
    pub all_devices_flashed: Option<bool>,
    pub uploaded_by: String,
    pub tag: String,
    pub status: Option<ReleaseTestStatus>,
    pub is_faulty: bool,
    pub device_family: String,
    pub device_type: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        has_many = "super::faulty_report::Entity",
        from = "Column::Id",
        to = "super::faulty_report::Column::ReleaseId"
    )]
    FaultyReports,
}

impl Related<super::faulty_report::Entity> for Entity {
    /// PURPOSE: Join path to faulty reports filed against this release.
    /// USE: Surface related reports when marking or listing a release.
    fn to() -> RelationDef {
        Relation::FaultyReports.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// PURPOSE: Deserialize the `artifacts` JSON column into typed metadata.
    /// USE: Call before flash/stage so workers iterate [`ArtifactInfo`] safely.
    /// Returns a serde error if the stored JSON does not match the DTO shape.
    pub fn artifacts_typed(&self) -> Result<Vec<ArtifactInfo>, serde_json::Error> {
        serde_json::from_value(self.artifacts.clone())
    }
}
