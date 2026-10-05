use sqlx::PgPool;
use uuid::Uuid;

use super::{
    AvailableRelayDevicesQuery,
    constants::MAX_PAGE_SIZE,
    error::DeviceRepositoryError,
    repository_types::{DevicePowerTarget, RelayIdentityTarget, RelayStateTarget},
};

pub(crate) async fn device_power_target(
    pool: &PgPool,
    device_id: &str,
) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError> {
    Ok(sqlx::query_as::<_, DevicePowerTarget>(
        r#"SELECT d."deviceFamily" AS device_family, d.power::text AS current_power,
             COALESCE(relay_dc."ipAddress", mapping."ipAddress") AS controller_ip,
             mapping.power_port, r."serialNumber" AS relay_serial,
             rc.id AS channel_id, rc."channelNumber" AS channel_number
           FROM devices d
           LEFT JOIN relay_channels rc ON rc."deviceId" = d."deviceId" AND rc."deletedAt" IS NULL
           LEFT JOIN relays r ON r.id = rc."relayId" AND r."deletedAt" IS NULL
           LEFT JOIN device_controllers relay_dc ON relay_dc."deviceControllerId" = r."deviceControllerId"
           LEFT JOIN LATERAL (
             SELECT dc."ipAddress",
               dc.mappings -> regexp_replace(lower(COALESCE(d."macAddress", d."deviceId")), '[:-]', '', 'g') ->> 'power' AS power_port
             FROM device_controllers dc
             WHERE dc.state::text = 'active' AND dc.status::text = 'approved'
               AND dc."deletedAt" IS NULL
               AND dc.mappings ? regexp_replace(lower(COALESCE(d."macAddress", d."deviceId")), '[:-]', '', 'g')
             ORDER BY (dc."deviceControllerId" = d."controllerId") DESC, dc."updatedAt" DESC LIMIT 1
           ) mapping ON true
           WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL
           ORDER BY rc."updatedAt" DESC NULLS LAST LIMIT 1"#,
    )
    .bind(device_id)
    .fetch_optional(pool)
    .await?)
}

pub(crate) async fn apply_device_power(
    pool: &PgPool,
    device_id: &str,
    channel_id: Option<Uuid>,
    state: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    if let Some(channel_id) = channel_id {
        sqlx::query(
            r#"UPDATE relay_channels SET state = $2, "updatedAt" = now()
               WHERE id = $1 AND "deletedAt" IS NULL"#,
        )
        .bind(channel_id)
        .bind(state)
        .execute(&mut *transaction)
        .await?;
    }
    let updated = sqlx::query(
        r#"UPDATE devices SET power = $2, "updatedAt" = now()
           WHERE "deviceId" = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(device_id)
    .bind(state)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    let channel = if let Some(channel_id) = channel_id {
        sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(rc) FROM relay_channels rc WHERE rc.id = $1"#,
        )
        .bind(channel_id)
        .fetch_optional(&mut *transaction)
        .await?
    } else {
        None
    };
    transaction.commit().await?;
    Ok((updated > 0).then(|| {
        channel.unwrap_or_else(|| serde_json::json!({"deviceId": device_id, "state": state}))
    }))
}

pub(crate) async fn relays_for_controller(
    pool: &PgPool,
    controller_id: &str,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT to_jsonb(r) || jsonb_build_object('relayChannels', COALESCE((
             SELECT jsonb_agg(to_jsonb(rc) || jsonb_build_object('device', CASE
               WHEN d."deviceId" IS NULL THEN NULL ELSE jsonb_build_object(
                 'deviceId', d."deviceId", 'macAddress', d."macAddress",
                 'deviceName', d."deviceName", 'ipAddress', d."ipAddress",
                 'deviceType', d."deviceType") END) ORDER BY rc."channelNumber")
             FROM relay_channels rc LEFT JOIN devices d ON d."deviceId" = rc."deviceId"
             WHERE rc."relayId" = r.id AND rc."deletedAt" IS NULL
           ), '[]'::jsonb))
           FROM relays r WHERE r."deviceControllerId" = $1 AND r."deletedAt" IS NULL
           ORDER BY r."createdAt" ASC"#,
    )
    .bind(controller_id)
    .fetch_all(pool)
    .await?)
}

pub(crate) async fn channels_for_relay(
    pool: &PgPool,
    relay_id: Uuid,
) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM relays WHERE id = $1 AND "deletedAt" IS NULL)"#,
    )
    .bind(relay_id)
    .fetch_one(pool)
    .await?;
    if !exists {
        return Ok(None);
    }
    Ok(Some(
        sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(rc) || jsonb_build_object('device', CASE
             WHEN d."deviceId" IS NULL THEN NULL ELSE jsonb_build_object(
               'deviceId', d."deviceId", 'macAddress', d."macAddress",
               'deviceName', d."deviceName", 'ipAddress', d."ipAddress",
               'deviceType', d."deviceType") END)
           FROM relay_channels rc LEFT JOIN devices d ON d."deviceId" = rc."deviceId"
           WHERE rc."relayId" = $1 AND rc."deletedAt" IS NULL
           ORDER BY rc."channelNumber" ASC"#,
        )
        .bind(relay_id)
        .fetch_all(pool)
        .await?,
    ))
}

pub(crate) async fn available_devices(
    pool: &PgPool,
    query: &AvailableRelayDevicesQuery,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(10).clamp(1, MAX_PAGE_SIZE);
    let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
    let generation = sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT CASE
             WHEN replace(lower(dc."deviceFamily"), ' ', '') LIKE '%gen3%' THEN 'gen3'
             WHEN replace(lower(dc."deviceFamily"), ' ', '') LIKE '%gen4%' THEN 'gen4'
             ELSE NULL END
           FROM relays r JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId"
           WHERE r.id = $1 AND r."deletedAt" IS NULL"#,
    )
    .bind(query.relay_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    let total_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM devices d
           WHERE d."deletedAt" IS NULL
             AND NOT EXISTS (SELECT 1 FROM relay_channels rc
               WHERE rc."deviceId" = d."deviceId" AND rc."deletedAt" IS NULL
                 AND ($1::uuid IS NULL OR rc."relayId" <> $1))
             AND ($2::text IS NULL OR d."deviceFamily" ILIKE '%' || $2 || '%'
               OR d."deviceType" ILIKE '%' || $2 || '%')"#,
    )
    .bind(query.relay_id)
    .bind(generation.as_deref())
    .fetch_one(pool)
    .await?;
    let devices = sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT jsonb_build_object(
             'deviceId', d."deviceId", 'macAddress', d."macAddress",
             'deviceName', d."deviceName", 'ipAddress', d."ipAddress",
             'deviceType', d."deviceType", 'deviceFamily', d."deviceFamily")
           FROM devices d WHERE d."deletedAt" IS NULL
             AND NOT EXISTS (SELECT 1 FROM relay_channels rc
               WHERE rc."deviceId" = d."deviceId" AND rc."deletedAt" IS NULL
                 AND ($1::uuid IS NULL OR rc."relayId" <> $1))
             AND ($2::text IS NULL OR d."deviceFamily" ILIKE '%' || $2 || '%'
               OR d."deviceType" ILIKE '%' || $2 || '%')
           ORDER BY d."deviceId" ASC LIMIT $3 OFFSET $4"#,
    )
    .bind(query.relay_id)
    .bind(generation.as_deref())
    .bind(i64::from(limit))
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(serde_json::json!({
        "devices": devices,
        "pagination": {
            "page": page, "limit": limit, "totalCount": total_count,
            "totalPages": u64::try_from(total_count).unwrap_or(0).div_ceil(u64::from(limit)),
        }
    }))
}

pub(crate) async fn state_target(
    pool: &PgPool,
    device_id: &str,
) -> Result<Option<RelayStateTarget>, DeviceRepositoryError> {
    Ok(sqlx::query_as::<_, RelayStateTarget>(
        r#"SELECT dc."ipAddress" AS controller_ip,
             r."serialNumber" AS relay_serial, rc.id AS channel_id,
             rc."channelNumber" AS channel_number
           FROM relay_channels rc
           JOIN relays r ON r.id = rc."relayId" AND r."deletedAt" IS NULL
           JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId" AND dc."deletedAt" IS NULL
           WHERE rc."deviceId" = $1 AND rc."deletedAt" IS NULL
           ORDER BY rc."updatedAt" DESC LIMIT 1"#,
    )
    .bind(device_id)
    .fetch_optional(pool)
    .await?)
}

pub(crate) async fn update_state(
    pool: &PgPool,
    channel_id: Uuid,
    state: &str,
) -> Result<(), DeviceRepositoryError> {
    sqlx::query(
        r#"UPDATE relay_channels SET state = $2, "updatedAt" = now()
           WHERE id = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(channel_id)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn identity_target(
    pool: &PgPool,
    relay_id: Uuid,
) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError> {
    Ok(sqlx::query_as::<_, RelayIdentityTarget>(
        r#"SELECT r."deviceControllerId" AS controller_id,
             dc."ipAddress" AS controller_ip, r."serialNumber" AS current_serial
           FROM relays r
           JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId" AND dc."deletedAt" IS NULL
           WHERE r.id = $1 AND r."deletedAt" IS NULL"#,
    )
    .bind(relay_id)
    .fetch_optional(pool)
    .await?)
}

pub(crate) async fn update_identity(
    pool: &PgPool,
    relay_id: Uuid,
    old_serial: &str,
    serial_number: &str,
    vendor_id: &str,
    product_id: &str,
) -> Result<bool, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let duplicate = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(SELECT 1 FROM relays
           WHERE "serialNumber" = $1 AND id <> $2 AND "deletedAt" IS NULL)"#,
    )
    .bind(serial_number)
    .bind(relay_id)
    .fetch_one(&mut *transaction)
    .await?;
    if duplicate {
        return Err(DeviceRepositoryError::Conflict(
            "Relay serial number is already assigned".to_owned(),
        ));
    }
    let updated = sqlx::query(
        r#"UPDATE relays SET "serialNumber" = $2, "vendorId" = $3,
                             "productId" = $4, "updatedAt" = now()
           WHERE id = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(relay_id)
    .bind(serial_number)
    .bind(vendor_id)
    .bind(product_id)
    .execute(&mut *transaction)
    .await?
    .rows_affected()
        > 0;
    if updated {
        sqlx::query(
            r#"UPDATE pending_relay_configs SET "relaySerial" = $2, "updatedAt" = now()
               WHERE "relaySerial" = $1 AND status::text = 'pending'"#,
        )
        .bind(old_serial)
        .bind(serial_number)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(updated)
}
