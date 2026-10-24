//! Build-folder mapping per device type (`device_type_folders` table).
//!
//! Ports `DeviceTypeFolder.ts`. Maps `(device_type, device_family)` to the NFS
//! folder name and default version used when scanning/uploading releases.

use sea_orm::entity::prelude::*;

/// Catalog entry linking a device type to its artifact folder conventions.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_type_folders", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub device_type: String,
    pub device_family: String,
    /// Directory name under the shared builds root.
    pub folder_name: String,
    /// Default version string when none is specified on upload/scan.
    pub default_version: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
