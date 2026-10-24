use super::*;

use crate::persistence::entity::{device, device_type_folder, release};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
    sea_query::OnConflict,
};

#[async_trait]
impl BuildRepository for PostgresDeviceRepository {
    async fn flag_build(
        &self,
        build_id: &str,
        request: &BuildFlagRequest,
    ) -> Result<Option<repository_types::BuildFlagResult>, DeviceRepositoryError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("devices::postgres::builds", "flag_build");
        build_store::flag_build(self.orm(), build_id, request.is_faulty).await
    }

    async fn update_build_status(
        &self,
        build_id: &str,
        status: &str,
    ) -> Result<Option<repository_types::BuildStatusResult>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::builds",
            "update_build_status",
        );
        build_store::update_build_status(self.orm(), build_id, status).await
    }

    async fn delete_build(
        &self,
        build_id: &str,
    ) -> Result<Option<BuildDeleteTarget>, DeviceRepositoryError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("devices::postgres::builds", "delete_build");
        build_store::delete_build(self.orm(), build_id).await
    }

    async fn finalize_build_upload(
        &self,
        request: &BuildUploadFinalization,
    ) -> Result<BuildUploadResult, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::builds",
            "finalize_build_upload",
        );
        build_store::finalize_upload(self.orm(), request).await
    }

    async fn init_build_upload(
        &self,
        file_count: i32,
        user_id: &str,
    ) -> Result<Uuid, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::builds",
            "init_build_upload",
        );
        build_store::init_upload(self.orm(), file_count, user_id).await
    }

    async fn mark_build_upload_started(
        &self,
        upload_id: &str,
        user_id: &str,
    ) -> Result<(), DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::builds",
            "mark_build_upload_started",
        );
        build_store::mark_upload_started(self.orm(), upload_id, user_id).await
    }

    async fn mark_build_upload_failed(&self, upload_id: &str) -> Result<(), DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::builds",
            "mark_build_upload_failed",
        );
        build_store::mark_upload_failed(self.orm(), upload_id).await
    }

    async fn build_filters(&self) -> Result<BuildFilters, DeviceRepositoryError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("devices::postgres::builds", "build_filters");
        build_store::build_filters(self.orm()).await
    }

    async fn build_by_id(
        &self,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("devices::postgres::builds", "build_by_id");
        build_store::build_by_id(self.orm(), build_id).await
    }

    async fn list_builds(
        &self,
        query: &BuildListQuery,
    ) -> Result<BuildList, DeviceRepositoryError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("devices::postgres::builds", "list_builds");
        build_store::list_builds(self.orm(), query).await
    }
}

#[async_trait]
impl DeviceArtifactRepository for PostgresDeviceRepository {
    async fn builds_for_device_type(
        &self,
        device_type: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::builds",
            "builds_for_device_type",
        );
        let folders = device_type_folder::Entity::find()
            .filter(device_type_folder::Column::DeviceType.eq(device_type))
            .all(self.orm())
            .await?;
        if folders.is_empty() {
            return Ok(Vec::new());
        }
        let folder_names: Vec<String> = folders.into_iter().map(|f| f.folder_name).collect();
        let folder_lower: Vec<String> = folder_names
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect();
        Ok(release::Entity::find()
            .order_by_desc(release::Column::CreatedAt)
            .all(self.orm())
            .await?
            .into_iter()
            .filter(|row| folder_lower.contains(&row.folder_name.to_ascii_lowercase()))
            .map(|row| {
                serde_json::json!({
                    "id": row.id,
                    "version": row.version,
                    "status": row.status,
                    "createdAt": row.created_at,
                    "lastScannedAt": row.last_scanned_at,
                    "isFaulty": row.is_faulty,
                })
            })
            .collect())
    }

    async fn configure_artifacts(
        &self,
        entries: &[DeviceTypeFolder],
    ) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::builds",
            "configure_artifacts",
        );
        let transaction = self.orm().begin().await?;
        let mut configured = Vec::with_capacity(entries.len());
        for entry in entries {
            let model = device_type_folder::ActiveModel {
                device_type: Set(entry.device_type.clone()),
                folder_name: Set(entry.folder_name.clone()),
                device_family: Set(entry.device_family.clone()),
                default_version: Set(entry.default_version.clone()),
            };
            let row = device_type_folder::Entity::insert(model)
                .on_conflict(
                    OnConflict::column(device_type_folder::Column::DeviceType)
                        .update_columns([
                            device_type_folder::Column::FolderName,
                            device_type_folder::Column::DefaultVersion,
                        ])
                        .to_owned(),
                )
                .exec_with_returning(&transaction)
                .await?;
            configured.push(DeviceTypeFolder {
                device_type: row.device_type,
                folder_name: row.folder_name,
                device_family: row.device_family,
                default_version: row.default_version,
            });
        }
        transaction.commit().await?;
        Ok(configured)
    }

    async fn artifact_folders(&self) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::builds",
            "artifact_folders",
        );
        Ok(device_type_folder::Entity::find()
            .order_by_asc(device_type_folder::Column::DeviceType)
            .all(self.orm())
            .await?
            .into_iter()
            .map(|row| DeviceTypeFolder {
                device_type: row.device_type,
                folder_name: row.folder_name,
                device_family: row.device_family,
                default_version: row.default_version,
            })
            .collect())
    }

    async fn artifact_folder_for_device(
        &self,
        device_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::builds",
            "artifact_folder_for_device",
        );
        let Some(dev) = device::Entity::find()
            .filter(device::Column::DeviceId.eq(device_id))
            .filter(device::Column::DeletedAt.is_null())
            .one(self.orm())
            .await?
        else {
            return Ok(None);
        };
        let Some(device_type) = dev.device_type else {
            return Ok(None);
        };
        Ok(device_type_folder::Entity::find_by_id(device_type)
            .one(self.orm())
            .await?
            .map(|f| f.folder_name))
    }
}
