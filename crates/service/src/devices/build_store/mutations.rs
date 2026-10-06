use sqlx::PgPool;

use super::super::{
    error::DeviceRepositoryError,
    repository_types::{BuildDeleteTarget, BuildFlagResult},
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
