use async_trait::async_trait;
use uuid::Uuid;

use super::*;

/// Owns build records, filtering, uploads, and build lifecycle operations.
#[async_trait]
pub(crate) trait BuildRepository: Send + Sync {
    async fn flag_build(
        &self,
        build_id: &str,
        request: &BuildFlagRequest,
    ) -> Result<Option<BuildFlagResult>, DeviceRepositoryError>;
    async fn delete_build(
        &self,
        build_id: &str,
    ) -> Result<Option<BuildDeleteTarget>, DeviceRepositoryError>;
    async fn finalize_build_upload(
        &self,
        request: &BuildUploadFinalization,
    ) -> Result<BuildUploadResult, DeviceRepositoryError>;
    async fn init_build_upload(
        &self,
        file_count: i32,
        user_id: &str,
    ) -> Result<Uuid, DeviceRepositoryError>;
    async fn mark_build_upload_started(
        &self,
        upload_id: &str,
        user_id: &str,
    ) -> Result<(), DeviceRepositoryError>;
    async fn mark_build_upload_failed(&self, upload_id: &str) -> Result<(), DeviceRepositoryError>;
    async fn build_filters(&self) -> Result<BuildFilters, DeviceRepositoryError>;
    async fn build_by_id(
        &self,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn list_builds(&self, query: &BuildListQuery)
    -> Result<BuildList, DeviceRepositoryError>;
}
