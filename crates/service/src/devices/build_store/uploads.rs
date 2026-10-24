//! Build upload init/start/fail/finalize persistence and alert JSON.
//!
//! Used by TUS/multipart upload handlers and `BuildRepository`. Ports Node
//! `BuildService` upload lifecycle against the releases table.

use std::path::Path;

use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use uuid::Uuid;

use crate::persistence::entity::{alert, device_type_folder, release, upload_build};

use super::super::{
    build_ingestion,
    error::DeviceRepositoryError,
    repository_types::{BuildUploadFinalization, BuildUploadResult},
};

/// PURPOSE: Ingest uploaded archive, insert release row, and build alert JSON.
/// USE: TUS/multipart completion after bytes are fully received.
pub(crate) async fn finalize_upload(
    orm: &DatabaseConnection,
    request: &BuildUploadFinalization,
) -> Result<BuildUploadResult, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::build_store::uploads", "finalize_upload");
    let upload_id = Uuid::parse_str(&request.upload_id)
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
    let generated_tag = format!("build-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let stored_tag = tag.unwrap_or(&generated_tag);
    let version = format!(
        "v{}{}",
        raw_version.trim_start_matches(['v', 'V']),
        if is_custom && tag.is_some() {
            format!("-{stored_tag}")
        } else {
            String::new()
        }
    );
    let folder = device_type_folder::Entity::find()
        .filter(device_type_folder::Column::DeviceType.eq(device_type))
        .one(orm)
        .await?
        .map(|row| (row.folder_name, row.device_family))
        .unwrap_or_else(|| (device_type.to_owned(), "unknown".to_owned()));
    let staging = request
        .artifacts_root
        .join(format!(".upload-{}", Uuid::new_v4()));
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
    let rootfs_archive_extension = request.rootfs_archive_extension.clone();
    let stored_archive_name = format!("{base_name}.zip");
    let mut artifacts = tokio::task::spawn_blocking(move || {
        build_ingestion::extract_zip(
            &source,
            &staging_for_extract,
            &expected_root,
            Some(&family),
            &rootfs_archive_extension,
            &stored_archive_name,
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
    let transaction = orm.begin().await?;
    let release_id = Uuid::new_v4();
    let now = Utc::now();
    let status = if is_custom {
        release::ReleaseTestStatus::Passed
    } else {
        release::ReleaseTestStatus::Untested
    };
    let build_type = if is_custom {
        release::BuildType::Custom
    } else {
        release::BuildType::Official
    };
    let release_model = match (release::ActiveModel {
        id: Set(release_id),
        folder_name: Set(folder.0.clone()),
        version: Set(version.clone()),
        artifacts: Set(serde_json::Value::Array(artifacts)),
        tag: Set(stored_tag.to_owned()),
        uploaded_by: Set(request.user_id.clone()),
        build_type: Set(build_type),
        device_family: Set(folder.1.clone()),
        device_type: Set(device_type.to_owned()),
        status: Set(Some(status)),
        is_faulty: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    })
    .insert(&transaction)
    .await
    {
        Ok(model) => model,
        Err(error) => {
            let _ = transaction.rollback().await;
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(error.into());
        }
    };
    let release = release_json(&release_model);

    let Some(upload) = upload_build::Entity::find_by_id(upload_id)
        .one(&transaction)
        .await?
    else {
        let _ = transaction.rollback().await;
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(DeviceRepositoryError::Internal(
            "Upload session not found".to_owned(),
        ));
    };
    let mut files = upload.files_array;
    files.push(filename.to_owned());
    let completed = upload.completed_files.unwrap_or(0) + 1;
    upload_build::ActiveModel {
        id: Set(upload.id),
        file_name: Set(Some(filename.to_owned())),
        status: Set(upload_build::UploadStatus::Completed),
        files_array: Set(files),
        completed_files: Set(Some(completed)),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(&transaction)
    .await?;

    let alert_model = alert::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set("ADMIN".to_owned()),
        title: Set("Build Upload Success".to_owned()),
        message: Set(format!("Build {version} uploaded successfully")),
        r#type: Set(alert::AlertType::Success),
        status: Set(alert::AlertStatus::Unread),
        is_read: Set(false),
        build_id: Set(Some(release_id.to_string())),
        data: Set(Some(serde_json::json!({
            "buildId": release_id,
            "version": version,
            "deviceType": device_type,
        }))),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&transaction)
    .await?;
    let alert = alert_json(&alert_model);

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

/// PURPOSE: Implement `init_upload` domain/persistence logic.
/// USE: Called as `init_upload` from handlers, repository impls, or sibling helpers.
pub(crate) async fn init_upload(
    orm: &DatabaseConnection,
    file_count: i32,
    user_id: &str,
) -> Result<Uuid, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::build_store::uploads", "init_upload");
    let upload_id = Uuid::new_v4();
    let now = Utc::now();
    upload_build::ActiveModel {
        id: Set(upload_id),
        status: Set(upload_build::UploadStatus::NotStarted),
        user_id: Set(user_id.to_owned()),
        files_array: Set(Vec::new()),
        total_files: Set(Some(file_count)),
        completed_files: Set(Some(0)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(orm)
    .await?;
    Ok(upload_id)
}

/// PURPOSE: Mutate persisted state for `mark upload started`.
/// USE: Update handlers and workers after side effects succeed.
pub(crate) async fn mark_upload_started(
    orm: &DatabaseConnection,
    upload_id: &str,
    user_id: &str,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::build_store::uploads",
        "mark_upload_started",
    );
    let upload_id = Uuid::parse_str(upload_id)
        .map_err(|_| DeviceRepositoryError::Internal("Upload session not found".to_owned()))?;
    let now = Utc::now();
    upload_build::Entity::update_many()
        .col_expr(
            upload_build::Column::Status,
            crate::persistence::pg_enum::pg_enum(&upload_build::UploadStatus::Uploading),
        )
        .col_expr(
            upload_build::Column::UserId,
            sea_orm::sea_query::Expr::value(user_id.to_owned()),
        )
        .col_expr(
            upload_build::Column::UpdatedAt,
            sea_orm::sea_query::Expr::value(now),
        )
        .filter(upload_build::Column::Id.eq(upload_id))
        .exec(orm)
        .await?;
    Ok(())
}

/// PURPOSE: Mutate persisted state for `mark upload failed`.
/// USE: Update handlers and workers after side effects succeed.
pub(crate) async fn mark_upload_failed(
    orm: &DatabaseConnection,
    upload_id: &str,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::build_store::uploads",
        "mark_upload_failed",
    );
    let upload_id = Uuid::parse_str(upload_id)
        .map_err(|_| DeviceRepositoryError::Internal("Upload session not found".to_owned()))?;
    let now = Utc::now();
    upload_build::Entity::update_many()
        .col_expr(
            upload_build::Column::Status,
            crate::persistence::pg_enum::pg_enum(&upload_build::UploadStatus::Failed),
        )
        .col_expr(
            upload_build::Column::UpdatedAt,
            sea_orm::sea_query::Expr::value(now),
        )
        .filter(upload_build::Column::Id.eq(upload_id))
        .exec(orm)
        .await?;
    Ok(())
}

/// PURPOSE: Serialize `release json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(super) fn release_json(model: &release::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "folderName": model.folder_name,
        "version": model.version,
        "artifacts": model.artifacts,
        "lastScannedAt": model.last_scanned_at,
        "testedDeviceId": model.tested_device_id,
        "buildType": match model.build_type {
            release::BuildType::Custom => "custom",
            release::BuildType::Official => "official",
        },
        "allDevicesFlashed": model.all_devices_flashed,
        "uploadedBy": model.uploaded_by,
        "tag": model.tag,
        "status": model.status.as_ref().map(|s| match s {
            release::ReleaseTestStatus::Untested => "untested",
            release::ReleaseTestStatus::Testing => "testing",
            release::ReleaseTestStatus::Passed => "passed",
            release::ReleaseTestStatus::Failed => "failed",
        }),
        "isFaulty": model.is_faulty,
        "deviceFamily": model.device_family,
        "deviceType": model.device_type,
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
    })
}

/// PURPOSE: Serialize `alert json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(super) fn alert_json(model: &alert::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "userId": model.user_id,
        "title": model.title,
        "message": model.message,
        "type": match model.r#type {
            alert::AlertType::Info => "info",
            alert::AlertType::Success => "success",
            alert::AlertType::Warning => "warning",
            alert::AlertType::Error => "error",
        },
        "status": match model.status {
            alert::AlertStatus::Unread => "unread",
            alert::AlertStatus::Read => "read",
        },
        "isRead": model.is_read,
        "buildId": model.build_id,
        "data": model.data,
        "createdAt": model.created_at,
    })
}
