//! Artifact-folder and builds-by-type capability trait.

use async_trait::async_trait;

use super::*;

/// Owns build lookup and artifact-folder configuration by device type.
#[async_trait]
pub(crate) trait DeviceArtifactRepository: Send + Sync {
    /// PURPOSE: Implement `builds_for_device_type` domain/persistence logic.
    /// USE: Called as `builds_for_device_type` from handlers, repository impls, or sibling helpers.
    async fn builds_for_device_type(
        &self,
        device_type: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Apply or confirm `configure artifacts` configuration.
    /// USE: Relay configuration handlers and timeout workers.
    async fn configure_artifacts(
        &self,
        entries: &[DeviceTypeFolder],
    ) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError>;
    /// PURPOSE: Implement `artifact_folders` domain/persistence logic.
    /// USE: Called as `artifact_folders` from handlers, repository impls, or sibling helpers.
    async fn artifact_folders(&self) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError>;
    /// PURPOSE: Implement `artifact_folder_for_device` domain/persistence logic.
    /// USE: Called as `artifact_folder_for_device` from handlers, repository impls, or sibling helpers.
    async fn artifact_folder_for_device(
        &self,
        device_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError>;
}
