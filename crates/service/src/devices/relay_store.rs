use sea_orm::{
    ColumnTrait, DatabaseBackend, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    Statement, TransactionTrait, sea_query::Expr,
};
use uuid::Uuid;

use crate::persistence::entity::{device, pending_relay_config, relay, relay_channel};

use super::{
    AvailableRelayDevicesQuery,
    constants::MAX_PAGE_SIZE,
    error::DeviceRepositoryError,
    repository_types::{DevicePowerTarget, RelayIdentityTarget, RelayStateTarget},
};

#[derive(Debug, FromQueryResult)]
struct JsonRow {
    value: serde_json::Value,
}

#[derive(Debug, FromQueryResult)]
struct ExistsRow {
    exists: bool,
}

#[derive(Debug, FromQueryResult)]
struct CountRow {
    count: i64,
}

#[derive(Debug, FromQueryResult)]
struct GenerationRow {
    generation: Option<String>,
}

fn parse_channel_state(state: &str) -> Result<relay_channel::ChannelState, DeviceRepositoryError> {
    match state {
        "on" => Ok(relay_channel::ChannelState::On),
        "off" => Ok(relay_channel::ChannelState::Off),
        other => Err(DeviceRepositoryError::Validation(format!(
            "Invalid relay channel state: {other}"
        ))),
    }
}

fn parse_device_power(state: &str) -> Result<device::DevicePower, DeviceRepositoryError> {
    match state {
        "on" => Ok(device::DevicePower::On),
        "off" => Ok(device::DevicePower::Off),
        other => Err(DeviceRepositoryError::Validation(format!(
            "Invalid device power state: {other}"
        ))),
    }
}

pub(crate) async fn device_power_target(
    orm: &DatabaseConnection,
    device_id: &str,
) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "device_power_target");
    // Complex LATERAL join — keep as raw SQL via SeaORM Statement.
    Ok(
        DevicePowerTarget::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
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
            [device_id.into()],
        ))
        .one(orm)
        .await?,
    )
}

pub(crate) async fn apply_device_power(
    orm: &DatabaseConnection,
    device_id: &str,
    channel_id: Option<Uuid>,
    state: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "apply_device_power");
    let channel_state = parse_channel_state(state)?;
    let power = parse_device_power(state)?;
    let transaction = orm.begin().await?;
    let now = chrono::Utc::now();
    if let Some(channel_id) = channel_id {
        relay_channel::Entity::update_many()
            .col_expr(
                relay_channel::Column::State,
                Expr::value(Some(channel_state)),
            )
            .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
            .filter(relay_channel::Column::Id.eq(channel_id))
            .filter(relay_channel::Column::DeletedAt.is_null())
            .exec(&transaction)
            .await?;
    }
    let updated = device::Entity::update_many()
        .col_expr(device::Column::Power, Expr::value(Some(power)))
        .col_expr(device::Column::UpdatedAt, Expr::value(now))
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .exec(&transaction)
        .await?
        .rows_affected;
    let channel = if let Some(channel_id) = channel_id {
        JsonRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT to_jsonb(rc) AS value FROM relay_channels rc WHERE rc.id = $1"#,
            [channel_id.into()],
        ))
        .one(&transaction)
        .await?
        .map(|row| row.value)
    } else {
        None
    };
    transaction.commit().await?;
    Ok((updated > 0).then(|| {
        channel.unwrap_or_else(|| serde_json::json!({"deviceId": device_id, "state": state}))
    }))
}

pub(crate) async fn relays_for_controller(
    orm: &DatabaseConnection,
    controller_id: &str,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "relays_for_controller");
    // Nested to_jsonb + jsonb_agg kept as raw SQL for API response shape.
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(r) || jsonb_build_object('relayChannels', COALESCE((
             SELECT jsonb_agg(to_jsonb(rc) || jsonb_build_object('device', CASE
               WHEN d."deviceId" IS NULL THEN NULL ELSE jsonb_build_object(
                 'deviceId', d."deviceId", 'macAddress', d."macAddress",
                 'deviceName', d."deviceName", 'ipAddress', d."ipAddress",
                 'deviceType', d."deviceType") END) ORDER BY rc."channelNumber")
             FROM relay_channels rc LEFT JOIN devices d ON d."deviceId" = rc."deviceId"
             WHERE rc."relayId" = r.id AND rc."deletedAt" IS NULL
           ), '[]'::jsonb)) AS value
           FROM relays r WHERE r."deviceControllerId" = $1 AND r."deletedAt" IS NULL
           ORDER BY r."createdAt" ASC"#,
        [controller_id.into()],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect())
}

pub(crate) async fn channels_for_relay(
    orm: &DatabaseConnection,
    relay_id: Uuid,
) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "channels_for_relay");
    let exists = relay::Entity::find_by_id(relay_id)
        .filter(relay::Column::DeletedAt.is_null())
        .one(orm)
        .await?
        .is_some();
    if !exists {
        return Ok(None);
    }
    Ok(Some(
        JsonRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT to_jsonb(rc) || jsonb_build_object('device', CASE
             WHEN d."deviceId" IS NULL THEN NULL ELSE jsonb_build_object(
               'deviceId', d."deviceId", 'macAddress', d."macAddress",
               'deviceName', d."deviceName", 'ipAddress', d."ipAddress",
               'deviceType', d."deviceType") END) AS value
           FROM relay_channels rc LEFT JOIN devices d ON d."deviceId" = rc."deviceId"
           WHERE rc."relayId" = $1 AND rc."deletedAt" IS NULL
           ORDER BY rc."channelNumber" ASC"#,
            [relay_id.into()],
        ))
        .all(orm)
        .await?
        .into_iter()
        .map(|row| row.value)
        .collect(),
    ))
}

pub(crate) async fn available_devices(
    orm: &DatabaseConnection,
    query: &AvailableRelayDevicesQuery,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "available_devices");
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(10).clamp(1, MAX_PAGE_SIZE);
    let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
    let generation = GenerationRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT CASE
             WHEN replace(lower(dc."deviceFamily"), ' ', '') LIKE '%gen3%' THEN 'gen3'
             WHEN replace(lower(dc."deviceFamily"), ' ', '') LIKE '%gen4%' THEN 'gen4'
             ELSE NULL END AS generation
           FROM relays r JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId"
           WHERE r.id = $1 AND r."deletedAt" IS NULL"#,
        [query.relay_id.into()],
    ))
    .one(orm)
    .await?
    .and_then(|row| row.generation);
    let total_count = CountRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT count(*) AS count FROM devices d
           WHERE d."deletedAt" IS NULL
             AND NOT EXISTS (SELECT 1 FROM relay_channels rc
               WHERE rc."deviceId" = d."deviceId" AND rc."deletedAt" IS NULL
                 AND ($1::uuid IS NULL OR rc."relayId" <> $1))
             AND ($2::text IS NULL OR d."deviceFamily" ILIKE '%' || $2 || '%'
               OR d."deviceType" ILIKE '%' || $2 || '%')"#,
        [query.relay_id.into(), generation.clone().into()],
    ))
    .one(orm)
    .await?
    .map(|row| row.count)
    .unwrap_or(0);
    let devices = JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT jsonb_build_object(
             'deviceId', d."deviceId", 'macAddress', d."macAddress",
             'deviceName', d."deviceName", 'ipAddress', d."ipAddress",
             'deviceType', d."deviceType", 'deviceFamily', d."deviceFamily") AS value
           FROM devices d WHERE d."deletedAt" IS NULL
             AND NOT EXISTS (SELECT 1 FROM relay_channels rc
               WHERE rc."deviceId" = d."deviceId" AND rc."deletedAt" IS NULL
                 AND ($1::uuid IS NULL OR rc."relayId" <> $1))
             AND ($2::text IS NULL OR d."deviceFamily" ILIKE '%' || $2 || '%'
               OR d."deviceType" ILIKE '%' || $2 || '%')
           ORDER BY d."deviceId" ASC LIMIT $3 OFFSET $4"#,
        [
            query.relay_id.into(),
            generation.into(),
            i64::from(limit).into(),
            offset.into(),
        ],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "devices": devices,
        "pagination": {
            "page": page, "limit": limit, "totalCount": total_count,
            "totalPages": u64::try_from(total_count).unwrap_or(0).div_ceil(u64::from(limit)),
        }
    }))
}

pub(crate) async fn state_target(
    orm: &DatabaseConnection,
    device_id: &str,
) -> Result<Option<RelayStateTarget>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_store", "state_target");
    Ok(
        RelayStateTarget::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT dc."ipAddress" AS controller_ip,
             r."serialNumber" AS relay_serial, rc.id AS channel_id,
             rc."channelNumber" AS channel_number
           FROM relay_channels rc
           JOIN relays r ON r.id = rc."relayId" AND r."deletedAt" IS NULL
           JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId" AND dc."deletedAt" IS NULL
           WHERE rc."deviceId" = $1 AND rc."deletedAt" IS NULL
           ORDER BY rc."updatedAt" DESC LIMIT 1"#,
            [device_id.into()],
        ))
        .one(orm)
        .await?,
    )
}

pub(crate) async fn update_state(
    orm: &DatabaseConnection,
    channel_id: Uuid,
    state: &str,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_store", "update_state");
    let channel_state = parse_channel_state(state)?;
    let now = chrono::Utc::now();
    relay_channel::Entity::update_many()
        .col_expr(
            relay_channel::Column::State,
            Expr::value(Some(channel_state)),
        )
        .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
        .filter(relay_channel::Column::Id.eq(channel_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .exec(orm)
        .await?;
    Ok(())
}

pub(crate) async fn identity_target(
    orm: &DatabaseConnection,
    relay_id: Uuid,
) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "identity_target");
    Ok(
        RelayIdentityTarget::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT r."deviceControllerId" AS controller_id,
             dc."ipAddress" AS controller_ip, r."serialNumber" AS current_serial
           FROM relays r
           JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId" AND dc."deletedAt" IS NULL
           WHERE r.id = $1 AND r."deletedAt" IS NULL"#,
            [relay_id.into()],
        ))
        .one(orm)
        .await?,
    )
}

pub(crate) async fn update_identity(
    orm: &DatabaseConnection,
    relay_id: Uuid,
    old_serial: &str,
    serial_number: &str,
    vendor_id: &str,
    product_id: &str,
) -> Result<bool, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "update_identity");
    let transaction = orm.begin().await?;
    let duplicate = ExistsRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT EXISTS(SELECT 1 FROM relays
           WHERE "serialNumber" = $1 AND id <> $2 AND "deletedAt" IS NULL) AS exists"#,
        [serial_number.into(), relay_id.into()],
    ))
    .one(&transaction)
    .await?
    .map(|row| row.exists)
    .unwrap_or(false);
    if duplicate {
        return Err(DeviceRepositoryError::Conflict(
            "Relay serial number is already assigned".to_owned(),
        ));
    }
    let now = chrono::Utc::now();
    let updated = relay::Entity::update_many()
        .col_expr(
            relay::Column::SerialNumber,
            Expr::value(serial_number.to_owned()),
        )
        .col_expr(
            relay::Column::VendorId,
            Expr::value(Some(vendor_id.to_owned())),
        )
        .col_expr(
            relay::Column::ProductId,
            Expr::value(Some(product_id.to_owned())),
        )
        .col_expr(relay::Column::UpdatedAt, Expr::value(now))
        .filter(relay::Column::Id.eq(relay_id))
        .filter(relay::Column::DeletedAt.is_null())
        .exec(&transaction)
        .await?
        .rows_affected
        > 0;
    if updated {
        pending_relay_config::Entity::update_many()
            .col_expr(
                pending_relay_config::Column::RelaySerial,
                Expr::value(serial_number.to_owned()),
            )
            .col_expr(pending_relay_config::Column::UpdatedAt, Expr::value(now))
            .filter(pending_relay_config::Column::RelaySerial.eq(old_serial))
            .filter(
                pending_relay_config::Column::Status
                    .eq(pending_relay_config::PendingRelayConfigStatus::Pending),
            )
            .exec(&transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(updated)
}
