use std::path::Path;

use sqlx::PgPool;

use super::super::{
    build_ingestion,
    error::DeviceRepositoryError,
    repository_types::{BuildUploadFinalization, BuildUploadResult},
};

pub(crate) async fn finalize_upload(
    pool: &PgPool,
    request: &BuildUploadFinalization,
) -> Result<BuildUploadResult, DeviceRepositoryError> {
    let upload_id = uuid::Uuid::parse_str(&request.upload_id)
        .map_err(|_| DeviceRepositoryError::Internal("Upload session not found".to_owned()))?;
    let filename = Path::new(&request.filename)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DeviceRepositoryError::Internal("Invalid upload filename".to_owned()))?;
    let base_name = filename
        .strip_suffix(".zip")
        .or_else(|| filename.strip_suffix(".ZIP"))
        .ok_or_else(|| {
            DeviceRepositoryError::Internal("Build upload must be a ZIP file".to_owned())
        })?;
    let (device_type, raw_version) = base_name.split_once("__").ok_or_else(|| {
        DeviceRepositoryError::Internal(
            "Invalid filename format. Expected deviceType__version.zip".to_owned(),
        )
    })?;
    if device_type.is_empty() || raw_version.is_empty() {
        return Err(DeviceRepositoryError::Internal(
            "Invalid filename format. Expected deviceType__version.zip".to_owned(),
        ));
    }
    let tag = request
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|tag| !tag.is_empty());
    if tag.is_some_and(|tag| {
        !tag.chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_/+".contains(character))
            || !tag
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
            || !tag
                .chars()
                .last()
                .is_some_and(|character| character.is_ascii_alphanumeric())
    }) {
        return Err(DeviceRepositoryError::Internal(
            "Invalid tag format".to_owned(),
        ));
    }
    let is_custom = request.is_custom;
    if is_custom && tag.is_none() {
        return Err(DeviceRepositoryError::Internal(
            "tag is required for custom builds".to_owned(),
        ));
    }
    let generated_tag = format!("build-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let tag = tag.unwrap_or(&generated_tag);
    let version = format!(
        "v{}{}",
        raw_version.trim_start_matches(['v', 'V']),
        if is_custom {
            format!("-{tag}")
        } else {
            String::new()
        }
    );
    let folder = sqlx::query_as::<_, (String, String)>(
        r#"SELECT "folderName", "deviceFamily" FROM device_type_folders WHERE "deviceType" = $1"#,
    )
    .bind(device_type)
    .fetch_optional(pool)
    .await?
    .unwrap_or_else(|| (device_type.to_owned(), "unknown".to_owned()));
    let staging = request
        .artifacts_root
        .join(format!(".upload-{}", uuid::Uuid::new_v4()));
    let destination = request.artifacts_root.join(&folder.0).join(&version);
    if tokio::fs::try_exists(&destination)
        .await
        .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?
    {
        return Err(DeviceRepositoryError::Conflict(format!(
            "Release already exists for {}/{}",
            folder.0, version
        )));
    }
    let source = request.source_path.clone();
    let staging_for_extract = staging.clone();
    let expected_root = base_name.to_owned();
    let family = folder.1.clone();
    let max_expanded = request.max_expanded_bytes;
    let mut artifacts = tokio::task::spawn_blocking(move || {
        build_ingestion::extract_zip(
            &source,
            &staging_for_extract,
            &expected_root,
            Some(&family),
            max_expanded,
        )
    })
    .await
    .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?
    .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
    for artifact in &mut artifacts {
        if let Some(path) = artifact.get_mut("path") {
            let name = path
                .as_str()
                .and_then(|path| Path::new(path).file_name())
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            *path =
                serde_json::Value::String(destination.join(name).to_string_lossy().into_owned());
        }
    }
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
    }
    let mut transaction = pool.begin().await?;
    let release_id = uuid::Uuid::new_v4();
    let release = sqlx::query_scalar::<_, serde_json::Value>(
        r#"INSERT INTO releases
               (id, "folderName", version, artifacts, tag, "uploadedBy", "buildType",
                "deviceFamily", "deviceType", "createdAt", "updatedAt")
           VALUES ($1, $2, $3, $4, $5, $6, $7::"enum_releases_buildType", $8, $9, now(), now())
           RETURNING to_jsonb(releases)"#,
    )
    .bind(release_id)
    .bind(&folder.0)
    .bind(&version)
    .bind(serde_json::Value::Array(artifacts))
    .bind(tag)
    .bind(&request.user_id)
    .bind(if is_custom { "custom" } else { "official" })
    .bind(&folder.1)
    .bind(device_type)
    .fetch_one(&mut *transaction)
    .await;
    let release = match release {
        Ok(release) => release,
        Err(error) => {
            let _ = transaction.rollback().await;
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(error.into());
        }
    };
    sqlx::query(
        r#"UPDATE upload_builds SET status = 'completed', "fileName" = $2,
               "filesArray" = array_append(COALESCE("filesArray", ARRAY[]::text[]), $2),
               "completedFiles" = COALESCE("completedFiles", 0) + 1, "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(upload_id)
    .bind(filename)
    .execute(&mut *transaction)
    .await?;
    let alert = sqlx::query_scalar::<_, serde_json::Value>(
        r#"INSERT INTO alerts
              (id, user_id, title, message, type, status, is_read, build_id,
               data, created_at, updated_at)
           VALUES (gen_random_uuid(), 'ADMIN', 'Build Upload Success',
                   'Build ' || $2 || ' uploaded successfully', 'success', 'unread', false,
                   $1::text, jsonb_build_object('buildId', $1, 'version', $2,
                                                'deviceType', $3), now(), now())
           RETURNING jsonb_build_object(
               'id', id, 'userId', user_id, 'title', title, 'message', message,
               'type', type::text, 'status', status::text, 'isRead', is_read,
               'buildId', build_id, 'data', data, 'createdAt', created_at)"#,
    )
    .bind(release_id)
    .bind(&version)
    .bind(device_type)
    .fetch_one(&mut *transaction)
    .await?;
    tokio::fs::rename(&staging, &destination)
        .await
        .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
    if let Err(error) = transaction.commit().await {
        let _ = tokio::fs::remove_dir_all(&destination).await;
        return Err(error.into());
    }
    let _ = tokio::fs::remove_file(&request.source_path).await;
    Ok(BuildUploadResult { release, alert })
}

pub(crate) async fn init_upload(
    pool: &PgPool,
    file_count: i32,
    user_id: &str,
) -> Result<uuid::Uuid, DeviceRepositoryError> {
    let upload_id = uuid::Uuid::new_v4();
    sqlx::query(r#"INSERT INTO upload_builds (id, status, "userId", "filesArray", "totalFiles", "completedFiles", "createdAt", "updatedAt") VALUES ($1, 'not_started', $2, $3, $4, 0, now(), now())"#)
        .bind(upload_id)
        .bind(user_id)
        .bind(Vec::<String>::new())
        .bind(file_count)
        .execute(pool)
        .await?;
    Ok(upload_id)
}

pub(crate) async fn mark_upload_started(
    pool: &PgPool,
    upload_id: &str,
    user_id: &str,
) -> Result<(), DeviceRepositoryError> {
    let upload_id = uuid::Uuid::parse_str(upload_id)
        .map_err(|_| DeviceRepositoryError::Internal("Upload session not found".to_owned()))?;
    sqlx::query(
        r#"UPDATE upload_builds SET status = 'uploading', "userId" = $2, "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(upload_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn mark_upload_failed(
    pool: &PgPool,
    upload_id: &str,
) -> Result<(), DeviceRepositoryError> {
    let upload_id = uuid::Uuid::parse_str(upload_id)
        .map_err(|_| DeviceRepositoryError::Internal("Upload session not found".to_owned()))?;
    sqlx::query(r#"UPDATE upload_builds SET status = 'failed', "updatedAt" = now() WHERE id = $1"#)
        .bind(upload_id)
        .execute(pool)
        .await?;
    Ok(())
}
