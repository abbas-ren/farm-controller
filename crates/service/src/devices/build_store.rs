use std::path::Path;

use sqlx::PgPool;

use super::{
    BuildFilters, BuildList, BuildListQuery, build_ingestion,
    error::DeviceRepositoryError,
    repository_types::{
        BuildDeleteTarget, BuildFlagResult, BuildUploadFinalization, BuildUploadResult,
    },
};

pub(crate) async fn flag_build(
    pool: &PgPool,
    build_id: &str,
    is_faulty: bool,
) -> Result<Option<BuildFlagResult>, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let target = sqlx::query_as::<_, (String, String)>(
        r#"UPDATE releases SET "isFaulty" = $2,
               status = CASE WHEN $2 THEN 'failed' ELSE 'passed' END,
               "updatedAt" = now() WHERE id::text = $1
           RETURNING version, "deviceType""#,
    )
    .bind(build_id)
    .bind(is_faulty)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((version, device_type)) = target else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let alert_type = if is_faulty { "warning" } else { "success" };
    let state = if is_faulty {
        "flagged as faulty"
    } else {
        "marked as healthy"
    };
    let alert = sqlx::query_scalar::<_, serde_json::Value>(
        r#"INSERT INTO alerts
              (id, user_id, title, message, type, status, is_read, build_id,
               data, created_at, updated_at)
           VALUES (gen_random_uuid(), 'ADMIN', 'Build Flagged',
                   'Build ' || $2 || ' ' || $5, $4, 'unread', false, $1,
                   jsonb_build_object('buildId', $1, 'version', $2,
                                      'deviceType', $3, 'isFaulty', $6), now(), now())
           RETURNING jsonb_build_object(
               'id', id, 'userId', user_id, 'title', title, 'message', message,
               'type', type::text, 'status', status::text, 'isRead', is_read,
               'buildId', build_id, 'data', data, 'createdAt', created_at)"#,
    )
    .bind(build_id)
    .bind(&version)
    .bind(&device_type)
    .bind(alert_type)
    .bind(state)
    .bind(is_faulty)
    .fetch_one(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Some(BuildFlagResult {
        version,
        device_type,
        alert,
    }))
}

pub(crate) async fn delete_build(
    pool: &PgPool,
    build_id: &str,
) -> Result<Option<BuildDeleteTarget>, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let target = sqlx::query_as::<_, (String, String, String, String)>(
        r#"DELETE FROM releases WHERE id::text = $1
           RETURNING "folderName" AS folder_name, version,
                     "deviceFamily" AS device_family, "deviceType" AS device_type"#,
    )
    .bind(build_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((folder_name, version, device_family, device_type)) = target else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let alert = sqlx::query_scalar::<_, serde_json::Value>(
        r#"INSERT INTO alerts
              (id, user_id, title, message, type, status, is_read, build_id,
               data, created_at, updated_at)
           VALUES (gen_random_uuid(), 'ADMIN', 'Build Deleted',
                   'Build ' || $2 || ' deleted', 'error', 'unread', false, $1,
                   jsonb_build_object('buildId', $1, 'version', $2, 'deviceType', $3),
                   now(), now())
           RETURNING jsonb_build_object(
               'id', id, 'userId', user_id, 'title', title, 'message', message,
               'type', type::text, 'status', status::text, 'isRead', is_read,
               'buildId', build_id, 'data', data, 'createdAt', created_at)"#,
    )
    .bind(build_id)
    .bind(&version)
    .bind(&device_type)
    .fetch_one(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Some(BuildDeleteTarget {
        folder_name,
        version,
        device_family,
        device_type,
        alert,
    }))
}

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

pub(crate) async fn build_filters(pool: &PgPool) -> Result<BuildFilters, DeviceRepositoryError> {
    let device_types = sqlx::query_scalar::<_, String>(
        r#"SELECT DISTINCT "deviceType" FROM releases WHERE "deviceType" IS NOT NULL ORDER BY "deviceType""#,
    )
    .fetch_all(pool)
    .await?;
    let device_families = sqlx::query_scalar::<_, String>(
        r#"SELECT DISTINCT "deviceFamily" FROM releases WHERE "deviceFamily" IS NOT NULL ORDER BY "deviceFamily""#,
    )
    .fetch_all(pool)
    .await?;
    Ok(BuildFilters {
        device_types,
        device_families,
    })
}

pub(crate) async fn build_by_id(
    pool: &PgPool,
    build_id: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT to_jsonb(release) FROM releases release WHERE id::text = $1"#,
    )
    .bind(build_id)
    .fetch_optional(pool)
    .await?)
}

pub(crate) async fn list_builds(
    pool: &PgPool,
    query: &BuildListQuery,
) -> Result<BuildList, DeviceRepositoryError> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(10).max(1);
    let flagged = query.flagged.as_deref().and_then(|value| match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    });
    let sort_by = query
        .sort_by
        .as_deref()
        .filter(|value| matches!(*value, "version" | "deviceFamily" | "deviceType" | "tag"))
        .unwrap_or("createdAt");
    let ascending = query.sort_order.as_deref() == Some("asc");
    let search = query.search.as_deref().filter(|value| !value.is_empty());
    let total_count = sqlx::query_scalar::<_, i64>(r#"SELECT count(*) FROM releases r WHERE ($1::text IS NULL OR r."deviceType" = $1) AND ($2::text IS NULL OR r."deviceFamily" = $2) AND ($3::boolean IS NULL OR r."isFaulty" = $3) AND ($4::text IS NULL OR r.version = $4) AND ($5::text IS NULL OR r."deviceType" ILIKE '%' || $5 || '%' OR r."deviceFamily" ILIKE '%' || $5 || '%' OR r.version ILIKE '%' || $5 || '%' OR r.tag ILIKE '%' || $5 || '%')"#)
        .bind(query.device_type.as_deref()).bind(query.device_family.as_deref()).bind(flagged)
        .bind(query.build_version.as_deref()).bind(search).fetch_one(pool).await?;
    let builds = sqlx::query_scalar::<_, serde_json::Value>(r#"SELECT to_jsonb(r) FROM releases r WHERE ($1::text IS NULL OR r."deviceType" = $1) AND ($2::text IS NULL OR r."deviceFamily" = $2) AND ($3::boolean IS NULL OR r."isFaulty" = $3) AND ($4::text IS NULL OR r.version = $4) AND ($5::text IS NULL OR r."deviceType" ILIKE '%' || $5 || '%' OR r."deviceFamily" ILIKE '%' || $5 || '%' OR r.version ILIKE '%' || $5 || '%' OR r.tag ILIKE '%' || $5 || '%') ORDER BY CASE WHEN $6 = 'createdAt' AND $7 THEN r."createdAt" END ASC, CASE WHEN $6 = 'createdAt' AND NOT $7 THEN r."createdAt" END DESC, CASE WHEN $6 = 'version' AND $7 THEN r.version END ASC, CASE WHEN $6 = 'version' AND NOT $7 THEN r.version END DESC, CASE WHEN $6 = 'deviceFamily' AND $7 THEN r."deviceFamily" END ASC, CASE WHEN $6 = 'deviceFamily' AND NOT $7 THEN r."deviceFamily" END DESC, CASE WHEN $6 = 'deviceType' AND $7 THEN r."deviceType" END ASC, CASE WHEN $6 = 'deviceType' AND NOT $7 THEN r."deviceType" END DESC, CASE WHEN $6 = 'tag' AND $7 THEN r.tag END ASC, CASE WHEN $6 = 'tag' AND NOT $7 THEN r.tag END DESC LIMIT $8 OFFSET $9"#)
        .bind(query.device_type.as_deref()).bind(query.device_family.as_deref()).bind(flagged)
        .bind(query.build_version.as_deref()).bind(search).bind(sort_by).bind(ascending)
        .bind(limit).bind((page - 1) * limit).fetch_all(pool).await?;
    Ok(BuildList {
        requested_count: builds.len() as u64,
        builds,
        total_count: total_count as u64,
        current_page: page as u64,
        total_pages: ((total_count + limit - 1) / limit).max(1) as u64,
    })
}
