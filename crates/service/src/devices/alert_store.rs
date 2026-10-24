use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
    sea_query::{Expr, extension::postgres::PgExpr},
};
use uuid::Uuid;

use crate::persistence::entity::alert;

use super::{
    error::DeviceRepositoryError,
    postgres::json_models::alert_json,
    AlertList,
};

fn admin_alert_filters(
    select: sea_orm::Select<alert::Entity>,
) -> sea_orm::Select<alert::Entity> {
    let thirty_days_ago = Utc::now() - chrono::Duration::days(30);
    select
        .filter(alert::Column::DeletedAt.is_null())
        .filter(
            Condition::any()
                .add(alert::Column::Status.ne(alert::AlertStatus::Read))
                .add(
                    Condition::all()
                        .add(alert::Column::Status.eq(alert::AlertStatus::Read))
                        .add(alert::Column::CreatedAt.gte(thirty_days_ago)),
                ),
        )
        .filter(
            Condition::any()
                .add(Expr::cust(
                    r#"(data->>'deviceId' IN (
                        SELECT d."deviceId" FROM devices d
                        WHERE d."deletedAt" IS NULL AND d.status::text = 'approved'
                    ))"#,
                ))
                .add(alert::Column::DeviceId.is_not_null())
                .add(alert::Column::DeviceControllerId.is_not_null())
                .add(alert::Column::BuildId.is_not_null())
                .add(alert::Column::FaultyReportId.is_not_null())
                .add(Expr::col(alert::Column::Title).ilike("%removed%")),
        )
}

fn order_admin_alerts(
    select: sea_orm::Select<alert::Entity>,
    sort_by: &str,
    descending: bool,
) -> sea_orm::Select<alert::Entity> {
    match (sort_by, descending) {
        ("title", true) => select.order_by_desc(alert::Column::Title),
        ("title", false) => select.order_by_asc(alert::Column::Title),
        ("type", true) => select.order_by_desc(alert::Column::Type),
        ("type", false) => select.order_by_asc(alert::Column::Type),
        ("status", true) => select.order_by_desc(alert::Column::Status),
        ("status", false) => select.order_by_asc(alert::Column::Status),
        ("isRead", true) => select.order_by_desc(alert::Column::IsRead),
        ("isRead", false) => select.order_by_asc(alert::Column::IsRead),
        ("updatedAt", true) => select.order_by_desc(alert::Column::UpdatedAt),
        ("updatedAt", false) => select.order_by_asc(alert::Column::UpdatedAt),
        (_, true) => select.order_by_desc(alert::Column::CreatedAt),
        (_, false) => select.order_by_asc(alert::Column::CreatedAt),
    }
}

pub(crate) async fn list_admin(
    orm: &DatabaseConnection,
    page: i64,
    limit: i64,
    sort_by: &str,
    descending: bool,
) -> Result<AlertList, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::alert_store", "list_admin");
    let total_data = admin_alert_filters(alert::Entity::find()).count(orm).await? as i64;
    let data = order_admin_alerts(admin_alert_filters(alert::Entity::find()), sort_by, descending)
        .limit(limit as u64)
        .offset(((page - 1) * limit) as u64)
        .all(orm)
        .await?
        .iter()
        .map(alert_json)
        .collect();

    Ok(AlertList {
        data,
        total_data,
        total_pages: (total_data + limit - 1) / limit,
        current_page: page,
        total_unread_count: None,
    })
}

fn list_all_filters(
    mut select: sea_orm::Select<alert::Entity>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> sea_orm::Select<alert::Entity> {
    select = select.filter(alert::Column::DeletedAt.is_null());
    if let Some(from) = from {
        select = select.filter(alert::Column::CreatedAt.gte(from));
    }
    if let Some(to) = to {
        select = select.filter(alert::Column::CreatedAt.lte(to));
    }
    select
}

pub(crate) async fn list_all(
    orm: &DatabaseConnection,
    page: i64,
    limit: i64,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> Result<AlertList, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::alert_store", "list_all");
    let total_data = list_all_filters(alert::Entity::find(), from, to)
        .count(orm)
        .await? as i64;
    let total_unread_count = list_all_filters(alert::Entity::find(), from, to)
        .filter(alert::Column::IsRead.eq(false))
        .count(orm)
        .await? as i64;
    let data = list_all_filters(alert::Entity::find(), from, to)
        .order_by_desc(alert::Column::CreatedAt)
        .limit(limit as u64)
        .offset(((page - 1) * limit) as u64)
        .all(orm)
        .await?
        .iter()
        .map(alert_json)
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
