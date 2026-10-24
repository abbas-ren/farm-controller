//! Build list filters, detail, and paginated inventory queries.
//!
//! Used by build HTTP list/detail handlers. Ports Node `BuildService` list.

use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
    sea_query::{Expr, extension::postgres::PgExpr},
};
use uuid::Uuid;

use crate::persistence::entity::release;

use super::super::{
    error::DeviceRepositoryError,
    postgres::json_models::release_json,
    BuildFilters, BuildList, BuildListQuery,
};

/// PURPOSE: Build SeaORM filter/order clauses for `build filters`.
/// USE: Shared by count and page queries.
pub(crate) async fn build_filters(
    orm: &DatabaseConnection,
) -> Result<BuildFilters, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::build_store::queries", "build_filters");
    let device_types = release::Entity::find()
        .filter(release::Column::DeviceType.is_not_null())
        .select_only()
        .column(release::Column::DeviceType)
        .distinct()
        .order_by_asc(release::Column::DeviceType)
        .into_tuple::<String>()
        .all(orm)
        .await?;
    let device_families = release::Entity::find()
        .filter(release::Column::DeviceFamily.is_not_null())
        .select_only()
        .column(release::Column::DeviceFamily)
        .distinct()
        .order_by_asc(release::Column::DeviceFamily)
        .into_tuple::<String>()
        .all(orm)
        .await?;
    Ok(BuildFilters {
        device_types,
        device_families,
    })
}

/// PURPOSE: Fetch a single record for `build by id`.
/// USE: Detail handlers and repository lookups.
pub(crate) async fn build_by_id(
    orm: &DatabaseConnection,
    build_id: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::build_store::queries", "build_by_id");
    let Ok(build_uuid) = Uuid::parse_str(build_id) else {
        return Ok(None);
    };
    Ok(release::Entity::find_by_id(build_uuid)
        .one(orm)
        .await?
        .map(|row| release_json(&row)))
}

/// PURPOSE: Build SeaORM filter/order clauses for `build list filters`.
/// USE: Shared by count and page queries.
fn build_list_filters(
    mut select: sea_orm::Select<release::Entity>,
    query: &BuildListQuery,
    flagged: Option<bool>,
) -> sea_orm::Select<release::Entity> {
    if let Some(device_type) = query.device_type.as_deref() {
        select = select.filter(release::Column::DeviceType.eq(device_type));
    }
    if let Some(device_family) = query.device_family.as_deref() {
        select = select.filter(release::Column::DeviceFamily.eq(device_family));
    }
    if let Some(flagged) = flagged {
        select = select.filter(release::Column::IsFaulty.eq(flagged));
    }
    if let Some(version) = query.build_version.as_deref() {
        select = select.filter(release::Column::Version.eq(version));
    }
    if let Some(search) = query.search.as_deref().filter(|value| !value.is_empty()) {
        let pattern = format!("%{search}%");
        select = select.filter(
            Condition::any()
                .add(Expr::col(release::Column::DeviceType).ilike(&pattern))
                .add(Expr::col(release::Column::DeviceFamily).ilike(&pattern))
                .add(Expr::col(release::Column::Version).ilike(&pattern))
                .add(Expr::col(release::Column::Tag).ilike(&pattern)),
        );
    }
    select
}

/// PURPOSE: Build SeaORM filter/order clauses for `order builds`.
/// USE: Shared by count and page queries.
fn order_builds(
    select: sea_orm::Select<release::Entity>,
    sort_by: &str,
    ascending: bool,
) -> sea_orm::Select<release::Entity> {
    match (sort_by, ascending) {
        ("version", true) => select.order_by_asc(release::Column::Version),
        ("version", false) => select.order_by_desc(release::Column::Version),
        ("deviceFamily", true) => select.order_by_asc(release::Column::DeviceFamily),
        ("deviceFamily", false) => select.order_by_desc(release::Column::DeviceFamily),
        ("deviceType", true) => select.order_by_asc(release::Column::DeviceType),
        ("deviceType", false) => select.order_by_desc(release::Column::DeviceType),
        ("tag", true) => select.order_by_asc(release::Column::Tag),
        ("tag", false) => select.order_by_desc(release::Column::Tag),
        (_, true) => select.order_by_asc(release::Column::CreatedAt),
        (_, false) => select.order_by_desc(release::Column::CreatedAt),
    }
}

/// PURPOSE: List/filter `list builds` rows for the API.
/// USE: HTTP list handlers and repository trait methods.
pub(crate) async fn list_builds(
    orm: &DatabaseConnection,
    query: &BuildListQuery,
) -> Result<BuildList, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::build_store::queries", "list_builds");
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

    let total_count =
        build_list_filters(release::Entity::find(), query, flagged).count(orm).await? as i64;
    let builds = order_builds(
        build_list_filters(release::Entity::find(), query, flagged),
        sort_by,
        ascending,
    )
    .limit(limit as u64)
    .offset(((page - 1) * limit) as u64)
    .all(orm)
    .await?
    .iter()
    .map(release_json)
    .collect::<Vec<_>>();

    Ok(BuildList {
        requested_count: builds.len() as u64,
        builds,
        total_count: total_count as u64,
        current_page: page as u64,
        total_pages: ((total_count + limit - 1) / limit).max(1) as u64,
    })
}
