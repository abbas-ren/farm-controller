use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{
    LegacyRelayChannelListQuery, LegacyRelayConfiguration, LegacyRelayConflictQuery,
    LegacyRelayListQuery, RelayConflictState,
    error::DeviceRepositoryError,
    repository_types::{RelayConfigurationResult, RelayControllerAction},
};

#[derive(sqlx::FromRow)]
struct LegacyConfigurationContext {
    channel_id: Uuid,
    channel_number: i32,
    controller_address: String,
    device_mac: String,
    device_generation: Option<String>,
    relay_serial: String,
}

#[derive(sqlx::FromRow)]
struct RemapContext {
    relay_id: Uuid,
    serial_number: String,
    controller_id: String,
    controller_address: String,
}

pub(crate) async fn list_relays(
    pool: &PgPool,
    query: &LegacyRelayListQuery,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
        r#"SELECT to_jsonb(r) FROM relays r
           WHERE r."deletedAt" IS NULL
             AND ($1::text IS NULL OR r."deviceControllerId" = $1)
           ORDER BY r."createdAt" ASC"#,
    )
    .bind(query.controller_id.as_deref())
    .fetch_all(pool)
    .await?)
}

pub(crate) async fn list_channels(
    pool: &PgPool,
    query: &LegacyRelayChannelListQuery,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
        r#"SELECT to_jsonb(rc) FROM relay_channels rc
           WHERE rc."deletedAt" IS NULL
             AND ($1::text IS NULL OR rc."deviceId" = $1)
             AND ($2::uuid IS NULL OR rc."relayId" = $2)
           ORDER BY rc."createdAt" ASC"#,
    )
    .bind(query.device_id.as_deref())
    .bind(query.relay_id)
    .fetch_all(pool)
    .await?)
}

pub(crate) async fn relay_by_id(
    pool: &PgPool,
    relay_id: Uuid,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
        r#"SELECT to_jsonb(r) FROM relays r
           WHERE r.id = $1 AND r."deletedAt" IS NULL"#,
    )
    .bind(relay_id)
    .fetch_optional(pool)
    .await?)
}

pub(crate) async fn channel_by_id(
    pool: &PgPool,
    channel_id: Uuid,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
        r#"SELECT to_jsonb(rc) FROM relay_channels rc
           WHERE rc.id = $1 AND rc."deletedAt" IS NULL"#,
    )
    .bind(channel_id)
    .fetch_optional(pool)
    .await?)
}

pub(crate) async fn delete_relay(
    pool: &PgPool,
    relay_id: Uuid,
) -> Result<bool, DeviceRepositoryError> {
    Ok(sqlx::query(
        r#"UPDATE relays SET "deletedAt" = now(), "updatedAt" = now()
           WHERE id = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(relay_id)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

pub(crate) async fn delete_channel(
    pool: &PgPool,
    channel_id: Uuid,
) -> Result<bool, DeviceRepositoryError> {
    Ok(sqlx::query(
        r#"UPDATE relay_channels SET "deletedAt" = now(), "updatedAt" = now()
           WHERE id = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(channel_id)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

pub(crate) async fn configure(
    pool: &PgPool,
    request: &LegacyRelayConfiguration,
) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
    let channel_id = request.id.ok_or_else(|| {
        DeviceRepositoryError::Validation("Relay channel id is required".to_owned())
    })?;
    let device_id = request
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DeviceRepositoryError::Validation("Device id is required".to_owned()))?;
    let mut transaction = pool.begin().await?;
    let context = sqlx::query_as::<_, LegacyConfigurationContext>(
        r#"SELECT rc.id AS channel_id, rc."channelNumber" AS channel_number,
             dc."ipAddress" AS controller_address,
             COALESCE(NULLIF(d."macAddress", ''), d."deviceId") AS device_mac,
             d."deviceType" AS device_generation, r."serialNumber" AS relay_serial
           FROM relay_channels rc
           JOIN relays r ON r.id = rc."relayId" AND r."deletedAt" IS NULL
           JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId" AND dc."deletedAt" IS NULL
           JOIN devices d ON d."deviceId" = $2 AND d."deletedAt" IS NULL
           WHERE rc.id = $1 AND rc."deletedAt" IS NULL
           FOR UPDATE OF rc"#,
    )
    .bind(channel_id)
    .bind(device_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| {
        DeviceRepositoryError::NotFound("Device, relay channel, or controller not found".to_owned())
    })?;
    sqlx::query(
        r#"UPDATE relay_channels SET "deviceId" = NULL, "updatedAt" = now()
           WHERE "deviceId" = $1 AND id <> $2 AND "deletedAt" IS NULL"#,
    )
    .bind(device_id)
    .bind(channel_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"UPDATE relay_channels SET "deviceId" = $2, "updatedAt" = now()
           WHERE id = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(channel_id)
    .bind(device_id)
    .execute(&mut *transaction)
    .await?;
    let channel = channel_json(&mut transaction, context.channel_id).await?;
    transaction.commit().await?;
    Ok(RelayConfigurationResult {
        channels: vec![channel],
        actions: vec![RelayControllerAction::Configure {
            controller_address: context.controller_address,
            device_mac: context.device_mac,
            relay_serial: context.relay_serial,
            channel_number: context.channel_number,
            device_generation: context.device_generation,
        }],
    })
}

pub(crate) async fn fresh(pool: &PgPool, relay_id: Uuid) -> Result<(), DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let relay = sqlx::query_as::<_, (String, String)>(
        r#"SELECT "serialNumber", "deviceControllerId" FROM relays
           WHERE id = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(relay_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| DeviceRepositoryError::NotFound("Relay not found".to_owned()))?;
    sqlx::query(
        r#"DELETE FROM relays WHERE "serialNumber" = $1
           AND "deviceControllerId" <> $2"#,
    )
    .bind(relay.0)
    .bind(relay.1)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub(crate) async fn remap(
    pool: &PgPool,
    relay_id: Uuid,
) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let target = sqlx::query_as::<_, RemapContext>(
        r#"SELECT r.id AS relay_id, r."serialNumber" AS serial_number,
             r."deviceControllerId" AS controller_id,
             dc."ipAddress" AS controller_address
           FROM relays r JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId" AND dc."deletedAt" IS NULL
           WHERE r.id = $1 AND r."deletedAt" IS NULL FOR UPDATE OF r"#,
    )
    .bind(relay_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| DeviceRepositoryError::NotFound("Relay or controller not found".to_owned()))?;
    let previous_relay = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM relays WHERE "serialNumber" = $1 AND id <> $2
           AND "deviceControllerId" <> $3 ORDER BY "createdAt" ASC LIMIT 1"#,
    )
    .bind(&target.serial_number)
    .bind(target.relay_id)
    .bind(&target.controller_id)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some(previous_relay) = previous_relay {
        sqlx::query(
            r#"UPDATE relay_channels SET "deletedAt" = NULL, "relayId" = $1,
                 "updatedAt" = now() WHERE "relayId" = $2"#,
        )
        .bind(target.relay_id)
        .bind(previous_relay)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(r#"DELETE FROM relays WHERE id = $1"#)
            .bind(previous_relay)
            .execute(&mut *transaction)
            .await?;
    }
    let mapped = sqlx::query_as::<_, (serde_json::Value, String, i32, Option<String>)>(
        r#"SELECT to_jsonb(rc), COALESCE(NULLIF(d."macAddress", ''), d."deviceId"),
             rc."channelNumber", d."deviceType"
           FROM relay_channels rc JOIN devices d
             ON d."deviceId" = rc."deviceId" AND d."deletedAt" IS NULL
           WHERE rc."relayId" = $1 AND rc."deletedAt" IS NULL
           ORDER BY rc."channelNumber" ASC"#,
    )
    .bind(target.relay_id)
    .fetch_all(&mut *transaction)
    .await?;
    let channels = mapped.iter().map(|row| row.0.clone()).collect();
    let actions = mapped
        .into_iter()
        .map(|(_, device_mac, channel_number, device_generation)| {
            RelayControllerAction::Configure {
                controller_address: target.controller_address.clone(),
                device_mac,
                relay_serial: target.serial_number.clone(),
                channel_number,
                device_generation,
            }
        })
        .collect();
    transaction.commit().await?;
    Ok(RelayConfigurationResult { channels, actions })
}

pub(crate) async fn check_conflicts(
    pool: &PgPool,
    query: &LegacyRelayConflictQuery,
) -> Result<RelayConflictState, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let serial_number = sqlx::query_scalar::<_, String>(
        r#"SELECT "serialNumber" FROM relays
           WHERE id = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(query.relay_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| DeviceRepositoryError::NotFound("Relay not found".to_owned()))?;
    let device_conflict = if let Some(device_id) = query.device_id.as_deref() {
        sqlx::query_scalar(
            r#"SELECT EXISTS(SELECT 1 FROM relay_channels
               WHERE "deviceId" = $1 AND "deletedAt" IS NULL
                 AND ("relayId" <> $2 OR "channelNumber" <> $3))"#,
        )
        .bind(device_id)
        .bind(query.relay_id)
        .bind(query.channel_number)
        .fetch_one(&mut *transaction)
        .await?
    } else {
        false
    };
    let channel_conflict = if let Some(device_id) = query.device_id.as_deref() {
        sqlx::query_scalar(
            r#"SELECT EXISTS(SELECT 1 FROM relay_channels
               WHERE "relayId" = $1 AND "channelNumber" = $2
                 AND "deviceId" <> $3 AND "deletedAt" IS NULL)"#,
        )
        .bind(query.relay_id)
        .bind(query.channel_number)
        .bind(device_id)
        .fetch_one(&mut *transaction)
        .await?
    } else {
        false
    };
    let duplicate_relays = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM relays WHERE "serialNumber" = $1 AND id <> $2"#,
    )
    .bind(serial_number)
    .bind(query.relay_id)
    .fetch_all(&mut *transaction)
    .await?;
    let relay_conflict = if duplicate_relays.is_empty() {
        false
    } else {
        let has_channels: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(SELECT 1 FROM relay_channels WHERE "relayId" = ANY($1))"#,
        )
        .bind(&duplicate_relays)
        .fetch_one(&mut *transaction)
        .await?;
        if !has_channels {
            sqlx::query(r#"DELETE FROM relays WHERE id = ANY($1)"#)
                .bind(&duplicate_relays)
                .execute(&mut *transaction)
                .await?;
        }
        has_channels
    };
    transaction.commit().await?;
    Ok(RelayConflictState {
        device_conflict,
        relay_conflict,
        channel_conflict,
    })
}

async fn channel_json(
    transaction: &mut Transaction<'_, Postgres>,
    channel_id: Uuid,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    Ok(
        sqlx::query_scalar(r#"SELECT to_jsonb(rc) FROM relay_channels rc WHERE rc.id = $1"#)
            .bind(channel_id)
            .fetch_one(&mut **transaction)
            .await?,
    )
}
