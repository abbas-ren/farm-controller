use sea_orm::{
    ColumnTrait, DatabaseBackend, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, Statement,
};
use uuid::Uuid;

use crate::persistence::entity::release;

use super::super::{BuildFilters, BuildList, BuildListQuery, error::DeviceRepositoryError};
use super::uploads::release_json;

#[derive(Debug, FromQueryResult)]
struct CountRow {
    count: i64,
}

#[derive(Debug, FromQueryResult)]
struct JsonRow {
    value: serde_json::Value,
}

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
    let search = query.search.as_deref().filter(|value| !value.is_empty());
    // Dynamic optional filters + CASE ORDER BY kept as Statement.
    let total_count = CountRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT count(*) AS count FROM releases r
           WHERE ($1::text IS NULL OR r."deviceType" = $1)
             AND ($2::text IS NULL OR r."deviceFamily" = $2)
             AND ($3::boolean IS NULL OR r."isFaulty" = $3)
             AND ($4::text IS NULL OR r.version = $4)
             AND ($5::text IS NULL OR r."deviceType" ILIKE '%' || $5 || '%'
                  OR r."deviceFamily" ILIKE '%' || $5 || '%'
                  OR r.version ILIKE '%' || $5 || '%'
                  OR r.tag ILIKE '%' || $5 || '%')"#,
        [
            query.device_type.clone().into(),
            query.device_family.clone().into(),
            flagged.into(),
            query.build_version.clone().into(),
            search.map(str::to_owned).into(),
        ],
    ))
    .one(orm)
    .await?
    .map(|row| row.count)
    .unwrap_or(0);
    let builds = JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(r) AS value FROM releases r
           WHERE ($1::text IS NULL OR r."deviceType" = $1)
             AND ($2::text IS NULL OR r."deviceFamily" = $2)
             AND ($3::boolean IS NULL OR r."isFaulty" = $3)
             AND ($4::text IS NULL OR r.version = $4)
             AND ($5::text IS NULL OR r."deviceType" ILIKE '%' || $5 || '%'
                  OR r."deviceFamily" ILIKE '%' || $5 || '%'
                  OR r.version ILIKE '%' || $5 || '%'
                  OR r.tag ILIKE '%' || $5 || '%')
           ORDER BY
             CASE WHEN $6 = 'createdAt' AND $7 THEN r."createdAt" END ASC,
             CASE WHEN $6 = 'createdAt' AND NOT $7 THEN r."createdAt" END DESC,
             CASE WHEN $6 = 'version' AND $7 THEN r.version END ASC,
             CASE WHEN $6 = 'version' AND NOT $7 THEN r.version END DESC,
             CASE WHEN $6 = 'deviceFamily' AND $7 THEN r."deviceFamily" END ASC,
             CASE WHEN $6 = 'deviceFamily' AND NOT $7 THEN r."deviceFamily" END DESC,
             CASE WHEN $6 = 'deviceType' AND $7 THEN r."deviceType" END ASC,
             CASE WHEN $6 = 'deviceType' AND NOT $7 THEN r."deviceType" END DESC,
             CASE WHEN $6 = 'tag' AND $7 THEN r.tag END ASC,
             CASE WHEN $6 = 'tag' AND NOT $7 THEN r.tag END DESC
           LIMIT $8 OFFSET $9"#,
        [
            query.device_type.clone().into(),
            query.device_family.clone().into(),
            flagged.into(),
            query.build_version.clone().into(),
            search.map(str::to_owned).into(),
            sort_by.into(),
            ascending.into(),
            limit.into(),
            ((page - 1) * limit).into(),
        ],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect::<Vec<_>>();
    Ok(BuildList {
        requested_count: builds.len() as u64,
        builds,
        total_count: total_count as u64,
        current_page: page as u64,
        total_pages: ((total_count + limit - 1) / limit).max(1) as u64,
    })
}
