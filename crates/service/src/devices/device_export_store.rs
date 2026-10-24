use sea_orm::{DatabaseBackend, DatabaseConnection, FromQueryResult, Statement};

use super::{DeviceCsvQuery, error::DeviceRepositoryError};

#[derive(Debug, FromQueryResult)]
struct JsonRow {
    value: serde_json::Value,
}

pub(crate) async fn export_devices(
    orm: &DatabaseConnection,
    query: &DeviceCsvQuery,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::device_export_store", "export_devices");
    let search = query.search.as_deref().map(str::trim).unwrap_or_default();
    // Nested interfaces jsonb_agg kept as Statement for CSV export shape.
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(device_row) || jsonb_build_object(
                   'interfaces', COALESCE((
                       SELECT jsonb_agg(to_jsonb(interface_row) ORDER BY interface_row.id)
                       FROM device_interfaces interface_row
                       WHERE interface_row."deviceId" = device_row."deviceId"
                   ), '[]'::jsonb)
               ) AS value
           FROM devices device_row
           WHERE device_row."deletedAt" IS NULL
             AND ($1 = '' OR device_row."deviceName" ILIKE '%' || $1 || '%'
                  OR device_row."deviceId" ILIKE '%' || $1 || '%'
                  OR device_row."deviceType" ILIKE '%' || $1 || '%'
                  OR device_row."macAddress" ILIKE '%' || $1 || '%')
             AND ($2::text IS NULL OR device_row."createdAt" >= $2::timestamptz)
             AND ($3::text IS NULL OR device_row."createdAt" <= $3::timestamptz)
           ORDER BY device_row."createdAt" DESC"#,
        [
            search.into(),
            query.from_date.clone().into(),
            query.to_date.clone().into(),
        ],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect())
}
