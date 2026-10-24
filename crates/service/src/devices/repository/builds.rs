//! Build inventory and upload lifecycle capability trait.
//!
//! Implemented by Postgres; mirrors Node `BuildService` surface.

use async_trait::async_trait;
use uuid::Uuid;

use super::*;
use crate::devices::repository_types::BuildStatusResult;

/// Owns build records, filtering, uploads, and build lifecycle operations.
#[async_trait]
pub(crate) trait BuildRepository: Send + Sync {
    /// PURPOSE: Flag a build and create an alert.
    /// USE: Flag-build HTTP handler.
    async fn flag_build(
        &self,
        build_id: &str,
        request: &BuildFlagRequest,
    ) -> Result<Option<BuildFlagResult>, DeviceRepositoryError>;
    /// PURPOSE: Update release status (e.g. faulty).
    /// USE: Update-build-status handler.
    async fn update_build_status(
        &self,
        build_id: &str,
        status: &str,
    ) -> Result<Option<BuildStatusResult>, DeviceRepositoryError>;
    /// PURPOSE: Soft-delete a build and return folder cleanup target.
    /// USE: Delete-build handler.
    async fn delete_build(
        &self,
        build_id: &str,
    ) -> Result<Option<BuildDeleteTarget>, DeviceRepositoryError>;
    /// PURPOSE: Create release row after upload lands on disk.
    /// USE: Upload completion path.
    async fn finalize_build_upload(
        &self,
        request: &BuildUploadFinalization,
    ) -> Result<BuildUploadResult, DeviceRepositoryError>;
    /// PURPOSE: Create an upload tracking row.
    /// USE: Upload init handler.
    async fn init_build_upload(
        &self,
        file_count: i32,
        user_id: &str,
    ) -> Result<Uuid, DeviceRepositoryError>;
    /// PURPOSE: Mark upload as started for progress UI.
    /// USE: Upload start handler.
    async fn mark_build_upload_started(
        &self,
        upload_id: &str,
        user_id: &str,
    ) -> Result<(), DeviceRepositoryError>;
    /// PURPOSE: Mark upload failed.
    /// USE: Upload error path.
    async fn mark_build_upload_failed(&self, upload_id: &str) -> Result<(), DeviceRepositoryError>;
    /// PURPOSE: Distinct filter values for build list UI.
    /// USE: Build-filters handler.
    async fn build_filters(&self) -> Result<BuildFilters, DeviceRepositoryError>;
    /// PURPOSE: Fetch one build as API JSON.
    /// USE: Build-detail handler.
    async fn build_by_id(
        &self,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Paginated build inventory.
    /// USE: List-builds handler.
    async fn list_builds(&self, query: &BuildListQuery)
    -> Result<BuildList, DeviceRepositoryError>;
}
