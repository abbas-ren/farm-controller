use sqlx::PgPool;

use super::{DeviceCsvQuery, error::DeviceRepositoryError};

pub(crate) async fn export_devices(
    pool: &PgPool,
    query: &DeviceCsvQuery,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let search = query.search.as_deref().map(str::trim).unwrap_or_default();
    Ok(sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT to_jsonb(device_row) || jsonb_build_object(
                   'interfaces', COALESCE((
                       SELECT jsonb_agg(to_jsonb(interface_row) ORDER BY interface_row.id)
                       FROM device_interfaces interface_row
                       WHERE interface_row."deviceId" = device_row."deviceId"
                   ), '[]'::jsonb)
               )
           FROM devices device_row
           WHERE device_row."deletedAt" IS NULL
             AND ($1 = '' OR device_row."deviceName" ILIKE '%' || $1 || '%'
                  OR device_row."deviceId" ILIKE '%' || $1 || '%'
                  OR device_row."deviceType" ILIKE '%' || $1 || '%'
                  OR device_row."macAddress" ILIKE '%' || $1 || '%')
             AND ($2::text IS NULL OR device_row."createdAt" >= $2::timestamptz)
             AND ($3::text IS NULL OR device_row."createdAt" <= $3::timestamptz)
           ORDER BY device_row."createdAt" DESC"#,
    )
    .bind(search)
    .bind(query.from_date.as_deref())
    .bind(query.to_date.as_deref())
    .fetch_all(pool)
    .await?)
}
