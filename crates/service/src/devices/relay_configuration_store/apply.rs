use std::collections::HashSet;

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection,
    EntityTrait, FromQueryResult, QueryFilter, Set, Statement, TransactionTrait, sea_query::Expr,
};
use serde_json::Value;
use uuid::Uuid;

use crate::persistence::entity::{
    device, device_controller, pending_relay_config, relay, relay_channel,
};

use super::super::{
    RelayChannelConfiguration,
    error::DeviceRepositoryError,
    repository_types::{RelayConfigurationResult, RelayControllerAction},
};
use super::mappings::{clear_gpio_mapping, upsert_gpio_mapping};

#[derive(Debug, FromQueryResult)]
struct ChannelContext {
    controller_address: String,
    mappings: Value,
    relay_serial: String,
    current_device_id: Option<String>,
    channel_number: i32,
    channel_state: String,
}

#[derive(Debug, FromQueryResult)]
struct DeviceContext {
    device_id: String,
    mac_address: Option<String>,
    device_generation: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct ExistingMapping {
    channel_id: Uuid,
    relay_serial: String,
    channel_number: i32,
}

#[derive(Debug, FromQueryResult)]
struct JsonRow {
    value: Value,
}

pub(crate) async fn configure(
    orm: &DatabaseConnection,
    configurations: &[RelayChannelConfiguration],
) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_configuration_store::apply", "configure");
    let cleared_channels: HashSet<Uuid> = configurations
        .iter()
        .filter(|configuration| normalized_device_id(configuration).is_none())
        .map(|configuration| configuration.channel_id)
        .collect();
    let mut ordered: Vec<&RelayChannelConfiguration> = configurations.iter().collect();
    // Clear channels first so a device can be reassigned within this same transaction.
    ordered.sort_by_key(|configuration| normalized_device_id(configuration).is_some());

    let transaction = orm.begin().await?;
    let mut channels = Vec::with_capacity(ordered.len());
    let mut actions = Vec::new();
    for configuration in ordered {
        channels.push(
            configure_channel(&transaction, configuration, &cleared_channels, &mut actions)
                .await?,
        );
    }
    transaction.commit().await?;
    Ok(RelayConfigurationResult { channels, actions })
}

async fn configure_channel(
    transaction: &impl ConnectionTrait,
    configuration: &RelayChannelConfiguration,
    cleared_channels: &HashSet<Uuid>,
    actions: &mut Vec<RelayControllerAction>,
) -> Result<Value, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::relay_configuration_store::apply",
        "configure_channel",
    );
    tracing::info!("Configuring relay channel: {:?}", configuration);
    let mut context = load_channel(transaction, configuration).await?;
    if !(0..=7).contains(&context.channel_number) {
        return Err(DeviceRepositoryError::Validation(
            "Relay channel is outside the supported hardware range".to_owned(),
        ));
    }
    let Some(device_id) = normalized_device_id(configuration) else {
        return load_channel_json(transaction, configuration.channel_id).await;
    };

    let device = load_device(transaction, device_id)
        .await?
        .ok_or_else(|| DeviceRepositoryError::NotFound(format!("Device {device_id} not found")))?;
    if let Some(existing) = ExistingMapping::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT rc.id AS channel_id, r."serialNumber" AS relay_serial,
             rc."channelNumber" AS channel_number
           FROM relay_channels rc
           JOIN relays r ON r.id = rc."relayId" AND r."deletedAt" IS NULL
           WHERE rc."deviceId" = $1 AND rc.id <> $2 AND rc."deletedAt" IS NULL
           LIMIT 1 FOR UPDATE OF rc"#,
        [device_id.into(), configuration.channel_id.into()],
    ))
    .one(transaction)
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

    let now = chrono::Utc::now();
    relay_channel::Entity::update_many()
        .col_expr(
            relay_channel::Column::DeviceId,
            Expr::value(Some(device_id.to_owned())),
        )
        .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
        .filter(relay_channel::Column::Id.eq(configuration.channel_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .exec(transaction)
        .await?;
    upsert_gpio_mapping(
        &mut context.mappings,
        &assigned_device_mac,
        configuration.gpio.as_deref(),
        configuration.gpio_default_level,
        configuration.relay_default_level,
    );
    save_mappings(transaction, configuration.relay_id, &context.mappings).await?;

    let power = match context.channel_state.as_str() {
        "on" => device::DevicePower::On,
        _ => device::DevicePower::Off,
    };
    device::Entity::update_many()
        .col_expr(device::Column::Power, Expr::value(Some(power)))
        .col_expr(device::Column::UpdatedAt, Expr::value(now))
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .exec(transaction)
        .await?;
    pending_relay_config::ActiveModel {
        id: Set(Uuid::new_v4()),
        relay_channel_id: Set(configuration.channel_id),
        relay_serial: Set(context.relay_serial.clone()),
        channel_no: Set(context.channel_number),
        device_mac: Set(assigned_device_mac.to_lowercase()),
        controller_address: Set(context.controller_address.clone()),
        device_gen: Set(device.device_generation.clone()),
        status: Set(pending_relay_config::PendingRelayConfigStatus::Pending),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(transaction)
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
    transaction: &impl ConnectionTrait,
    configuration: &RelayChannelConfiguration,
) -> Result<ChannelContext, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_configuration_store::apply", "load_channel");
    ChannelContext::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
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
        [configuration.relay_id.into(), configuration.channel_id.into()],
    ))
    .one(transaction)
    .await?
    .ok_or_else(|| {
        DeviceRepositoryError::NotFound(format!(
            "Relay channel {} not found for relay {}",
            configuration.channel_id, configuration.relay_id
        ))
    })
}

async fn load_device(
    transaction: &impl ConnectionTrait,
    device_id: &str,
) -> Result<Option<DeviceContext>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_configuration_store::apply", "load_device");
    Ok(device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .one(transaction)
        .await?
        .map(|row| DeviceContext {
            device_id: row.device_id,
            mac_address: row.mac_address,
            device_generation: row.device_type,
        }))
}

async fn save_mappings(
    transaction: &impl ConnectionTrait,
    relay_id: Uuid,
    mappings: &Value,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_configuration_store::apply", "save_mappings");
    let Some(relay_row) = relay::Entity::find_by_id(relay_id).one(transaction).await? else {
        return Ok(());
    };
    let now = chrono::Utc::now();
    device_controller::Entity::update_many()
        .col_expr(
            device_controller::Column::Mappings,
            Expr::value(mappings.clone()),
        )
        .col_expr(device_controller::Column::UpdatedAt, Expr::value(now))
        .filter(device_controller::Column::DeviceControllerId.eq(relay_row.device_controller_id))
        .exec(transaction)
        .await?;
    Ok(())
}

async fn load_channel_json(
    transaction: &impl ConnectionTrait,
    channel_id: Uuid,
) -> Result<Value, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::relay_configuration_store::apply",
        "load_channel_json",
    );
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(rc) AS value FROM relay_channels rc WHERE rc.id = $1"#,
        [channel_id.into()],
    ))
    .one(transaction)
    .await?
    .ok_or_else(|| DeviceRepositoryError::NotFound("Relay channel not found".to_owned()))?
    .value)
}

fn normalized_device_id(configuration: &RelayChannelConfiguration) -> Option<&str> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::relay_configuration_store::apply",
        "normalized_device_id",
    );
    configuration
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|device_id| !device_id.is_empty())
}

fn device_mac(device: &DeviceContext) -> String {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_configuration_store::apply", "device_mac");
    device
        .mac_address
        .clone()
        .filter(|mac| !mac.trim().is_empty())
        .unwrap_or_else(|| device.device_id.clone())
}
