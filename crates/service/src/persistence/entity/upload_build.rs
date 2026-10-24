//! Multi-file build upload sessions (`upload_builds` table).
//!
//! Ports `UploadBuild.ts` / `BuildService` upload tracking. Progress is tracked
//! per session until all files land and a release can be materialized.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Progress of an upload session.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_upload_builds_status"
)]
pub enum UploadStatus {
    #[sea_orm(string_value = "not_started")]
    NotStarted,
    #[sea_orm(string_value = "uploading")]
    Uploading,
    #[sea_orm(string_value = "failed")]
    Failed,
    #[sea_orm(string_value = "completed")]
    Completed,
}

/// One user-initiated multi-file build upload.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "upload_builds", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub file_name: Option<String>,
    pub status: UploadStatus,
    pub user_id: String,
    /// Names of files expected/received in this session.
    pub files_array: Vec<String>,
    pub total_files: Option<i32>,
    pub completed_files: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
