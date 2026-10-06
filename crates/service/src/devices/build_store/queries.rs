use sqlx::PgPool;

use super::super::{BuildFilters, BuildList, BuildListQuery, error::DeviceRepositoryError};

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
