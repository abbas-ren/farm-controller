use async_trait::async_trait;

use super::*;

/// Owns build lookup and artifact-folder configuration by device type.
#[async_trait]
pub(crate) trait DeviceArtifactRepository: Send + Sync {
    async fn builds_for_device_type(
        &self,
        device_type: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn configure_artifacts(
        &self,
        entries: &[DeviceTypeFolder],
    ) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError>;
    async fn artifact_folders(&self) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError>;
    async fn artifact_folder_for_device(
        &self,
        device_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError>;
}
