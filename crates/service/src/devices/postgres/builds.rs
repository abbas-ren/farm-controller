use super::*;

#[async_trait]
impl BuildRepository for PostgresDeviceRepository {
    async fn flag_build(
        &self,
        build_id: &str,
        request: &BuildFlagRequest,
    ) -> Result<Option<repository_types::BuildFlagResult>, DeviceRepositoryError> {
        build_store::flag_build(&self.pool, build_id, request.is_faulty).await
    }

    async fn delete_build(
        &self,
        build_id: &str,
    ) -> Result<Option<BuildDeleteTarget>, DeviceRepositoryError> {
        build_store::delete_build(&self.pool, build_id).await
    }

    async fn finalize_build_upload(
        &self,
        request: &BuildUploadFinalization,
    ) -> Result<BuildUploadResult, DeviceRepositoryError> {
        build_store::finalize_upload(&self.pool, request).await
    }

    async fn init_build_upload(
        &self,
        file_count: i32,
        user_id: &str,
    ) -> Result<Uuid, DeviceRepositoryError> {
        build_store::init_upload(&self.pool, file_count, user_id).await
    }

    async fn mark_build_upload_started(
        &self,
        upload_id: &str,
        user_id: &str,
    ) -> Result<(), DeviceRepositoryError> {
        build_store::mark_upload_started(&self.pool, upload_id, user_id).await
    }

    async fn mark_build_upload_failed(&self, upload_id: &str) -> Result<(), DeviceRepositoryError> {
        build_store::mark_upload_failed(&self.pool, upload_id).await
    }

    async fn build_filters(&self) -> Result<BuildFilters, DeviceRepositoryError> {
        build_store::build_filters(&self.pool).await
    }

    async fn build_by_id(
        &self,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        build_store::build_by_id(&self.pool, build_id).await
    }

    async fn list_builds(
        &self,
        query: &BuildListQuery,
    ) -> Result<BuildList, DeviceRepositoryError> {
        build_store::list_builds(&self.pool, query).await
    }
}

#[async_trait]
impl DeviceArtifactRepository for PostgresDeviceRepository {
    async fn builds_for_device_type(
        &self,
        device_type: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT jsonb_build_object(
                 'id', r.id, 'version', r.version, 'status', r.status,
                 'createdAt', r."createdAt", 'lastScannedAt', r."lastScannedAt",
                 'isFaulty', r."isFaulty")
               FROM releases r
               JOIN device_type_folders f ON r."folderName" ILIKE f."folderName"
               WHERE f."deviceType" = $1
               ORDER BY r."createdAt" DESC"#,
        )
        .bind(device_type)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn configure_artifacts(
        &self,
        entries: &[DeviceTypeFolder],
    ) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let mut configured = Vec::with_capacity(entries.len());
        for entry in entries {
            let row = sqlx::query_as::<_, (String, String, String, String)>(
                r#"INSERT INTO device_type_folders
                     ("deviceType", "folderName", "deviceFamily", "defaultVersion")
                   VALUES ($1, $2, $3, $4)
                   ON CONFLICT ("deviceType") DO UPDATE SET
                     "folderName" = EXCLUDED."folderName",
                     "defaultVersion" = EXCLUDED."defaultVersion"
                   RETURNING "deviceType", "folderName", "deviceFamily", "defaultVersion""#,
            )
            .bind(&entry.device_type)
            .bind(&entry.folder_name)
            .bind(&entry.device_family)
            .bind(&entry.default_version)
            .fetch_one(&mut *transaction)
            .await?;
            configured.push(DeviceTypeFolder {
                device_type: row.0,
                folder_name: row.1,
                device_family: row.2,
                default_version: row.3,
            });
        }
        transaction.commit().await?;
        Ok(configured)
    }

    async fn artifact_folders(&self) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, (String, String, String, String)>(
            r#"SELECT "deviceType", "folderName", "deviceFamily", "defaultVersion"
               FROM device_type_folders ORDER BY "deviceType""#,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| DeviceTypeFolder {
            device_type: row.0,
            folder_name: row.1,
            device_family: row.2,
            default_version: row.3,
        })
        .collect())
    }

    async fn artifact_folder_for_device(
        &self,
        device_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, String>(
            r#"SELECT f."folderName" FROM devices d
               JOIN device_type_folders f ON f."deviceType" = d."deviceType"
               WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL"#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?)
    }
}
