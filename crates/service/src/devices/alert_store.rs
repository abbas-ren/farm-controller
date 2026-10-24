use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, DatabaseBackend, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    Statement, sea_query::Expr,
};
use uuid::Uuid;

use crate::persistence::entity::alert;

use super::{AlertList, error::DeviceRepositoryError};

#[derive(Debug, FromQueryResult)]
struct CountRow {
    count: i64,
}

#[derive(Debug, FromQueryResult)]
struct JsonRow {
    value: serde_json::Value,
}

pub(crate) async fn list_admin(
    orm: &DatabaseConnection,
    page: i64,
    limit: i64,
    sort_by: &str,
    descending: bool,
) -> Result<AlertList, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::alert_store", "list_admin");
    let total_data = CountRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT COUNT(*) AS count
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
        [],
    ))
    .one(orm)
    .await?
    .map(|row| row.count)
    .unwrap_or(0);

    let data = JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT jsonb_build_object(
               'id', a.id, 'userId', a.user_id, 'title', a.title, 'message', a.message,
               'data', a.data, 'type', a.type::text, 'status', a.status::text,
               'isRead', a.is_read, 'readAt', a.read_at, 'createdAt', a.created_at,
               'updatedAt', a.updated_at, 'buildId', a.build_id,
               'faultyReportId', a.faulty_report_id, 'deviceId', a.device_id,
               'deviceControllerId', a.device_controller_id) AS value
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
        [
            limit.into(),
            ((page - 1) * limit).into(),
            descending.into(),
            sort_by.into(),
        ],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect();

    Ok(AlertList {
        data,
        total_data,
        total_pages: (total_data + limit - 1) / limit,
        current_page: page,
        total_unread_count: None,
    })
}

pub(crate) async fn list_all(
    orm: &DatabaseConnection,
    page: i64,
    limit: i64,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> Result<AlertList, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::alert_store", "list_all");
    let total_data = CountRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT COUNT(*) AS count FROM alerts
           WHERE deleted_at IS NULL
             AND ($1::timestamptz IS NULL OR created_at >= $1)
             AND ($2::timestamptz IS NULL OR created_at <= $2)"#,
        [from.into(), to.into()],
    ))
    .one(orm)
    .await?
    .map(|row| row.count)
    .unwrap_or(0);
    let total_unread_count = CountRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT COUNT(*) AS count FROM alerts
           WHERE deleted_at IS NULL AND is_read = false
             AND ($1::timestamptz IS NULL OR created_at >= $1)
             AND ($2::timestamptz IS NULL OR created_at <= $2)"#,
        [from.into(), to.into()],
    ))
    .one(orm)
    .await?
    .map(|row| row.count)
    .unwrap_or(0);
    let data = JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT jsonb_build_object(
               'id', a.id, 'userId', a.user_id, 'title', a.title, 'message', a.message,
               'data', a.data, 'type', a.type::text, 'status', a.status::text,
               'isRead', a.is_read, 'readAt', a.read_at, 'createdAt', a.created_at,
               'updatedAt', a.updated_at, 'buildId', a.build_id,
               'faultyReportId', a.faulty_report_id, 'deviceId', a.device_id,
               'deviceControllerId', a.device_controller_id) AS value
           FROM alerts a
           WHERE a.deleted_at IS NULL
             AND ($3::timestamptz IS NULL OR a.created_at >= $3)
             AND ($4::timestamptz IS NULL OR a.created_at <= $4)
           ORDER BY a.created_at DESC
            LIMIT $1 OFFSET $2"#,
        [
            limit.into(),
            ((page - 1) * limit).into(),
            from.into(),
            to.into(),
        ],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect();

    Ok(AlertList {
        data,
        total_data,
        total_pages: (total_data + limit - 1) / limit,
        current_page: page,
        total_unread_count: Some(total_unread_count),
    })
}

pub(crate) async fn mark_read(
    orm: &DatabaseConnection,
    alert_id: &str,
) -> Result<bool, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::alert_store", "mark_read");
    let Ok(id) = Uuid::parse_str(alert_id) else {
        return Ok(false);
    };
    let now = chrono::Utc::now();
    let result = alert::Entity::update_many()
        .col_expr(alert::Column::Status, Expr::value(alert::AlertStatus::Read))
        .col_expr(alert::Column::IsRead, Expr::value(true))
        .col_expr(alert::Column::ReadAt, Expr::value(Some(now)))
        .col_expr(alert::Column::UpdatedAt, Expr::value(now))
        .filter(alert::Column::Id.eq(id))
        .filter(alert::Column::DeletedAt.is_null())
        .exec(orm)
        .await?;
    Ok(result.rows_affected > 0)
}

pub(crate) async fn mark_all_read(orm: &DatabaseConnection) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::alert_store", "mark_all_read");
    let now = chrono::Utc::now();
    alert::Entity::update_many()
        .col_expr(alert::Column::Status, Expr::value(alert::AlertStatus::Read))
        .col_expr(alert::Column::IsRead, Expr::value(true))
        .col_expr(alert::Column::ReadAt, Expr::value(Some(now)))
        .col_expr(alert::Column::UpdatedAt, Expr::value(now))
        .filter(alert::Column::IsRead.eq(false))
        .filter(alert::Column::DeletedAt.is_null())
        .exec(orm)
        .await?;
    Ok(())
}
