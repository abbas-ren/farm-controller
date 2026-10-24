//! SeaORM queries for farm alerts (list, mark-read).
//!
//! Invoked by `AlertRepository` Postgres impls and alert HTTP handlers.
//! Ports the listing / read-state logic from Node `AlertsService` /
//! `AlertNotificationService`, including the admin filter that keeps unread
//! alerts plus recent read ones and only surfaces alerts tied to live inventory.

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

/// PURPOSE: Apply admin-dashboard alert visibility rules (soft-delete, unread/recent-read, inventory linkage).
/// USE: Shared by admin list count and page queries.
fn admin_alert_filters(
    select: sea_orm::Select<alert::Entity>,
) -> sea_orm::Select<alert::Entity> {
    let thirty_days_ago = Utc::now() - chrono::Duration::days(30);
    select
        // Soft-deleted alerts never appear in either admin or user lists.
        .filter(alert::Column::DeletedAt.is_null())
        .filter(
            // Unread always; read only if created within the last 30 days (legacy AlertsService window).
            Condition::any()
                .add(alert::Column::Status.ne(alert::AlertStatus::Read))
                .add(
                    Condition::all()
                        .add(alert::Column::Status.eq(alert::AlertStatus::Read))
                        .add(alert::Column::CreatedAt.gte(thirty_days_ago)),
                ),
        )
        .filter(
            // Keep alerts that reference approved devices, controllers, builds, faulty reports, or removal titles.
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

/// PURPOSE: Order admin alert rows by a UI sort key.
/// USE: Admin alerts list handler after filters are applied.
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

/// PURPOSE: Paginated admin alert list with legacy visibility filters.
/// USE: `GET` admin alerts API / `AlertRepository::admin_alerts`.
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

/// PURPOSE: Soft-delete + optional created-at window for the non-admin alert feed.
/// USE: Shared by `list_all` count, unread count, and page queries.
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

/// PURPOSE: Paginated alert feed with unread total for the date window.
/// USE: `GET` all-alerts API / `AlertRepository::all_alerts`.
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

/// PURPOSE: Mark a single alert read (status, isRead, readAt).
/// USE: Mark-read handlers; returns false when id is invalid or row missing.
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

/// PURPOSE: Mark every unread non-deleted alert as read.
/// USE: Mark-all-read admin/user action.
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
