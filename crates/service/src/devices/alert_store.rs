use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::{AlertList, error::DeviceRepositoryError};

pub(crate) async fn list_admin(
    pool: &PgPool,
    page: i64,
    limit: i64,
    sort_by: &str,
    descending: bool,
) -> Result<AlertList, DeviceRepositoryError> {
    let total_data = sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*)
           FROM alerts a
           WHERE a.deleted_at IS NULL
             AND (a.status::text <> 'read'
                  OR (a.status::text = 'read' AND a.created_at >= now() - interval '30 days'))
             AND (
                 a.data->>'deviceId' IN (
                     SELECT d."deviceId" FROM devices d
                     WHERE d."deletedAt" IS NULL AND d.status::text = 'approved'
                 )
                 OR a.device_id IS NOT NULL
                 OR a.device_controller_id IS NOT NULL
                 OR a.build_id IS NOT NULL
                 OR a.faulty_report_id IS NOT NULL
                 OR a.title ILIKE '%removed%'
             )"#,
    )
    .fetch_one(pool)
    .await?;

    let data = sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT jsonb_build_object(
               'id', a.id, 'userId', a.user_id, 'title', a.title, 'message', a.message,
               'data', a.data, 'type', a.type::text, 'status', a.status::text,
               'isRead', a.is_read, 'readAt', a.read_at, 'createdAt', a.created_at,
               'updatedAt', a.updated_at, 'buildId', a.build_id,
               'faultyReportId', a.faulty_report_id, 'deviceId', a.device_id,
               'deviceControllerId', a.device_controller_id)
           FROM alerts a
           WHERE a.deleted_at IS NULL
             AND (a.status::text <> 'read'
                  OR (a.status::text = 'read' AND a.created_at >= now() - interval '30 days'))
             AND (
                 a.data->>'deviceId' IN (
                     SELECT d."deviceId" FROM devices d
                     WHERE d."deletedAt" IS NULL AND d.status::text = 'approved'
                 )
                 OR a.device_id IS NOT NULL
                 OR a.device_controller_id IS NOT NULL
                 OR a.build_id IS NOT NULL
                 OR a.faulty_report_id IS NOT NULL
                 OR a.title ILIKE '%removed%'
             )
           ORDER BY
             CASE WHEN $3 THEN CASE $4
                 WHEN 'title' THEN to_jsonb(a.title)
                 WHEN 'type' THEN to_jsonb(a.type::text)
                 WHEN 'status' THEN to_jsonb(a.status::text)
                 WHEN 'isRead' THEN to_jsonb(a.is_read)
                 WHEN 'updatedAt' THEN to_jsonb(a.updated_at)
                 ELSE to_jsonb(a.created_at)
             END END DESC,
             CASE WHEN NOT $3 THEN CASE $4
                 WHEN 'title' THEN to_jsonb(a.title)
                 WHEN 'type' THEN to_jsonb(a.type::text)
                 WHEN 'status' THEN to_jsonb(a.status::text)
                 WHEN 'isRead' THEN to_jsonb(a.is_read)
                 WHEN 'updatedAt' THEN to_jsonb(a.updated_at)
                 ELSE to_jsonb(a.created_at)
             END END ASC
            LIMIT $1 OFFSET $2"#,
    )
    .bind(limit)
    .bind((page - 1) * limit)
    .bind(descending)
    .bind(sort_by)
    .fetch_all(pool)
    .await?;

    Ok(AlertList {
        data,
        total_data,
        total_pages: (total_data + limit - 1) / limit,
        current_page: page,
        total_unread_count: None,
    })
}

pub(crate) async fn list_all(
    pool: &PgPool,
    page: i64,
    limit: i64,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> Result<AlertList, DeviceRepositoryError> {
    let total_data = sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM alerts
           WHERE deleted_at IS NULL
             AND ($1::timestamptz IS NULL OR created_at >= $1)
             AND ($2::timestamptz IS NULL OR created_at <= $2)"#,
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;
    let total_unread_count = sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM alerts
           WHERE deleted_at IS NULL AND is_read = false
             AND ($1::timestamptz IS NULL OR created_at >= $1)
             AND ($2::timestamptz IS NULL OR created_at <= $2)"#,
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;
    let data = sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT jsonb_build_object(
               'id', a.id, 'userId', a.user_id, 'title', a.title, 'message', a.message,
               'data', a.data, 'type', a.type::text, 'status', a.status::text,
               'isRead', a.is_read, 'readAt', a.read_at, 'createdAt', a.created_at,
               'updatedAt', a.updated_at, 'buildId', a.build_id,
               'faultyReportId', a.faulty_report_id, 'deviceId', a.device_id,
               'deviceControllerId', a.device_controller_id)
           FROM alerts a
           WHERE a.deleted_at IS NULL
             AND ($3::timestamptz IS NULL OR a.created_at >= $3)
             AND ($4::timestamptz IS NULL OR a.created_at <= $4)
           ORDER BY a.created_at DESC
            LIMIT $1 OFFSET $2"#,
    )
    .bind(limit)
    .bind((page - 1) * limit)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    Ok(AlertList {
        data,
        total_data,
        total_pages: (total_data + limit - 1) / limit,
        current_page: page,
        total_unread_count: Some(total_unread_count),
    })
}

pub(crate) async fn mark_read(
    pool: &PgPool,
    alert_id: &str,
) -> Result<bool, DeviceRepositoryError> {
    Ok(sqlx::query(
        r#"UPDATE alerts
           SET status = 'read', is_read = true, read_at = now(), updated_at = now()
           WHERE id::text = $1 AND deleted_at IS NULL"#,
    )
    .bind(alert_id)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

pub(crate) async fn mark_all_read(pool: &PgPool) -> Result<(), DeviceRepositoryError> {
    sqlx::query(
        r#"UPDATE alerts
           SET status = 'read', is_read = true, read_at = now(), updated_at = now()
           WHERE is_read = false AND deleted_at IS NULL"#,
    )
    .execute(pool)
    .await?;
    Ok(())
}
