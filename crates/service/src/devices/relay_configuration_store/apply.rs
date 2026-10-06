use std::collections::HashSet;

use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::super::{
    RelayChannelConfiguration,
    error::DeviceRepositoryError,
    repository_types::{RelayConfigurationResult, RelayControllerAction},
};
use super::mappings::{clear_gpio_mapping, upsert_gpio_mapping};

#[derive(sqlx::FromRow)]
struct ChannelContext {
    controller_address: String,
    mappings: Value,
    relay_serial: String,
    current_device_id: Option<String>,
    channel_number: i32,
    channel_state: String,
}

#[derive(sqlx::FromRow)]
struct DeviceContext {
    device_id: String,
    mac_address: Option<String>,
    device_generation: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ExistingMapping {
    channel_id: Uuid,
    relay_serial: String,
    channel_number: i32,
}

pub(crate) async fn configure(
    pool: &PgPool,
    configurations: &[RelayChannelConfiguration],
) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
    let cleared_channels: HashSet<Uuid> = configurations
        .iter()
        .filter(|configuration| normalized_device_id(configuration).is_none())
        .map(|configuration| configuration.channel_id)
        .collect();
    let mut ordered: Vec<&RelayChannelConfiguration> = configurations.iter().collect();
    // Clear channels first so a device can be reassigned within this same transaction.
    ordered.sort_by_key(|configuration| normalized_device_id(configuration).is_some());

    let mut transaction = pool.begin().await?;
    let mut channels = Vec::with_capacity(ordered.len());
    let mut actions = Vec::new();
    for configuration in ordered {
        channels.push(
            configure_channel(
                &mut transaction,
                configuration,
                &cleared_channels,
                &mut actions,
            )
            .await?,
        );
    }
    transaction.commit().await?;
    Ok(RelayConfigurationResult { channels, actions })
}

async fn configure_channel(
    transaction: &mut Transaction<'_, Postgres>,
    configuration: &RelayChannelConfiguration,
    cleared_channels: &HashSet<Uuid>,
    actions: &mut Vec<RelayControllerAction>,
) -> Result<Value, DeviceRepositoryError> {
    let mut context = load_channel(transaction, configuration).await?;
    if !(0..=7).contains(&context.channel_number) {
        return Err(DeviceRepositoryError::Validation(
            "Relay channel is outside the supported hardware range".to_owned(),
        ));
    }
    let Some(device_id) = normalized_device_id(configuration) else {
        if let Some(previous_device_id) = context.current_device_id.as_deref() {
            if let Some(previous_device) = load_device(transaction, previous_device_id).await? {
                let device_mac = device_mac(&previous_device);
                actions.push(RelayControllerAction::Remove {
                    controller_address: context.controller_address.clone(),
                    device_mac: device_mac.clone(),
                    relay_serial: context.relay_serial.clone(),
                    channel_number: context.channel_number,
                });
                clear_gpio_mapping(&mut context.mappings, &device_mac);
                save_mappings(transaction, configuration.relay_id, &context.mappings).await?;
            }
            sqlx::query(
                r#"UPDATE relay_channels SET "deviceId" = NULL, "updatedAt" = now()
                   WHERE id = $1 AND "deletedAt" IS NULL"#,
            )
            .bind(configuration.channel_id)
            .execute(&mut **transaction)
            .await?;
        }
        return load_channel_json(transaction, configuration.channel_id).await;
    };

    let device = load_device(transaction, device_id)
        .await?
        .ok_or_else(|| DeviceRepositoryError::NotFound(format!("Device {device_id} not found")))?;
    if let Some(existing) = sqlx::query_as::<_, ExistingMapping>(
        r#"SELECT rc.id AS channel_id, r."serialNumber" AS relay_serial,
             rc."channelNumber" AS channel_number
           FROM relay_channels rc
           JOIN relays r ON r.id = rc."relayId" AND r."deletedAt" IS NULL
           WHERE rc."deviceId" = $1 AND rc.id <> $2 AND rc."deletedAt" IS NULL
           LIMIT 1 FOR UPDATE OF rc"#,
    )
    .bind(device_id)
    .bind(configuration.channel_id)
    .fetch_optional(&mut **transaction)
    .await?
        && !cleared_channels.contains(&existing.channel_id)
    {
        return Err(DeviceRepositoryError::Conflict(format!(
            "Device is already mapped to {} - Channel {}",
            existing.relay_serial,
            existing.channel_number + 1
        )));
    }

    let assigned_device_mac = device_mac(&device);
    if context.current_device_id.as_deref() == Some(device_id) {
        upsert_gpio_mapping(
            &mut context.mappings,
            &assigned_device_mac,
            configuration.gpio.as_deref(),
            configuration.gpio_default_level,
            configuration.relay_default_level,
        );
        save_mappings(transaction, configuration.relay_id, &context.mappings).await?;
        return load_channel_json(transaction, configuration.channel_id).await;
    }

    if let Some(previous_device_id) = context.current_device_id.as_deref()
        && let Some(previous_device) = load_device(transaction, previous_device_id).await?
    {
        let previous_mac = device_mac(&previous_device);
        actions.push(RelayControllerAction::Remove {
            controller_address: context.controller_address.clone(),
            device_mac: previous_mac.clone(),
            relay_serial: context.relay_serial.clone(),
            channel_number: context.channel_number,
        });
        clear_gpio_mapping(&mut context.mappings, &previous_mac);
    }

    sqlx::query(
        r#"UPDATE relay_channels SET "deviceId" = $2, "updatedAt" = now()
           WHERE id = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(configuration.channel_id)
    .bind(device_id)
    .execute(&mut **transaction)
    .await?;
    upsert_gpio_mapping(
        &mut context.mappings,
        &assigned_device_mac,
        configuration.gpio.as_deref(),
        configuration.gpio_default_level,
        configuration.relay_default_level,
    );
    save_mappings(transaction, configuration.relay_id, &context.mappings).await?;
    sqlx::query(
        r#"UPDATE devices SET power = $2::"enum_devices_power", "updatedAt" = now()
           WHERE "deviceId" = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(device_id)
    .bind(&context.channel_state)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        r#"INSERT INTO pending_relay_configs (
             id, "relayChannelId", "relaySerial", "channelNo", "deviceMac",
             "controllerAddress", "deviceGen", status, "createdAt", "updatedAt"
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', now(), now())"#,
    )
    .bind(Uuid::new_v4())
    .bind(configuration.channel_id)
    .bind(&context.relay_serial)
    .bind(context.channel_number)
    .bind(assigned_device_mac.to_lowercase())
    .bind(&context.controller_address)
    .bind(device.device_generation.as_deref())
    .execute(&mut **transaction)
    .await?;
    actions.push(RelayControllerAction::Configure {
        controller_address: context.controller_address,
        device_mac: assigned_device_mac,
        relay_serial: context.relay_serial,
        channel_number: context.channel_number,
        device_generation: device.device_generation,
    });
    load_channel_json(transaction, configuration.channel_id).await
}

async fn load_channel(
    transaction: &mut Transaction<'_, Postgres>,
    configuration: &RelayChannelConfiguration,
) -> Result<ChannelContext, DeviceRepositoryError> {
    sqlx::query_as::<_, ChannelContext>(
        r#"SELECT dc."ipAddress" AS controller_address,
             COALESCE(dc.mappings, '{}'::jsonb) AS mappings,
             r."serialNumber" AS relay_serial, rc."deviceId" AS current_device_id,
             rc."channelNumber" AS channel_number,
             COALESCE(rc.state::text, 'off') AS channel_state
           FROM relays r
           JOIN relay_channels rc ON rc."relayId" = r.id AND rc."deletedAt" IS NULL
           JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId" AND dc."deletedAt" IS NULL
           WHERE r.id = $1 AND rc.id = $2 AND r."deletedAt" IS NULL
           FOR UPDATE OF rc, dc"#,
    )
    .bind(configuration.relay_id)
    .bind(configuration.channel_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| {
        DeviceRepositoryError::NotFound(format!(
            "Relay channel {} not found for relay {}",
            configuration.channel_id, configuration.relay_id
        ))
    })
}

async fn load_device(
    transaction: &mut Transaction<'_, Postgres>,
    device_id: &str,
) -> Result<Option<DeviceContext>, DeviceRepositoryError> {
    Ok(sqlx::query_as::<_, DeviceContext>(
        r#"SELECT "deviceId" AS device_id, "macAddress" AS mac_address,
             "deviceType" AS device_generation
           FROM devices WHERE "deviceId" = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await?)
}

async fn save_mappings(
    transaction: &mut Transaction<'_, Postgres>,
    relay_id: Uuid,
    mappings: &Value,
) -> Result<(), DeviceRepositoryError> {
    sqlx::query(
        r#"UPDATE device_controllers SET mappings = $2, "updatedAt" = now()
           WHERE "deviceControllerId" = (
             SELECT "deviceControllerId" FROM relays WHERE id = $1
           )"#,
    )
    .bind(relay_id)
    .bind(sqlx::types::Json(mappings))
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn load_channel_json(
    transaction: &mut Transaction<'_, Postgres>,
    channel_id: Uuid,
) -> Result<Value, DeviceRepositoryError> {
    sqlx::query_scalar(r#"SELECT to_jsonb(rc) FROM relay_channels rc WHERE rc.id = $1"#)
        .bind(channel_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(Into::into)
}

fn normalized_device_id(configuration: &RelayChannelConfiguration) -> Option<&str> {
    configuration
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|device_id| !device_id.is_empty())
}

fn device_mac(device: &DeviceContext) -> String {
    device
        .mac_address
        .clone()
        .filter(|mac| !mac.trim().is_empty())
        .unwrap_or_else(|| device.device_id.clone())
}
