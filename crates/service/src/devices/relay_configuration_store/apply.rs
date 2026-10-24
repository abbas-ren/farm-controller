//! Persist relay channel configuration and compute edge controller actions.
//!
//! Used by configure-relay HTTP handlers before outbound edge commands.

use std::collections::HashSet;

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, LockType},
};
use serde_json::Value;
use uuid::Uuid;

use crate::persistence::entity::{
    device, device_controller, pending_relay_config, relay, relay_channel,
};

use super::super::{
    RelayChannelConfiguration,
    error::DeviceRepositoryError,
    postgres::json_models::relay_channel_json,
    repository_types::{RelayConfigurationResult, RelayControllerAction},
};
use super::mappings::{clear_gpio_mapping, upsert_gpio_mapping};

/// Domain/DTO type `ChannelContext` used across device stores and handlers.
struct ChannelContext {
    controller_address: String,
    mappings: Value,
    relay_serial: String,
    current_device_id: Option<String>,
    channel_number: i32,
    channel_state: String,
}

/// Domain/DTO type `DeviceContext` used across device stores and handlers.
struct DeviceContext {
    device_id: String,
    mac_address: Option<String>,
    device_generation: Option<String>,
}

/// Domain/DTO type `ExistingMapping` used across device stores and handlers.
struct ExistingMapping {
    channel_id: Uuid,
    relay_serial: String,
    channel_number: i32,
}

/// PURPOSE: Apply one or many channel↔device mappings and collect edge actions.
/// USE: Configure-relay HTTP handler after validation.
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

/// PURPOSE: Apply or confirm `configure channel` configuration.
/// USE: Relay configuration handlers and timeout workers.
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
    if let Some(existing) = load_existing_mapping(transaction, device_id, configuration.channel_id)
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
        .col_expr(
            device::Column::Power,
            crate::persistence::pg_enum::pg_enum(&power),
        )
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

/// PURPOSE: Load or save `load existing mapping`.
/// USE: Internal store helpers during multi-step transactions.
async fn load_existing_mapping(
    transaction: &impl ConnectionTrait,
    device_id: &str,
    channel_id: Uuid,
) -> Result<Option<ExistingMapping>, DeviceRepositoryError> {
    // Lock the channel row alone — find_also_related uses LEFT JOIN and Postgres
    // rejects FOR UPDATE on the nullable related side.
    let Some(channel) = relay_channel::Entity::find()
        .filter(relay_channel::Column::DeviceId.eq(device_id))
        .filter(relay_channel::Column::Id.ne(channel_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .lock(LockType::Update)
        .one(transaction)
        .await?
    else {
        return Ok(None);
    };
    let Some(relay_row) = relay::Entity::find_by_id(channel.relay_id)
        .filter(relay::Column::DeletedAt.is_null())
        .one(transaction)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(ExistingMapping {
        channel_id: channel.id,
        relay_serial: relay_row.serial_number,
        channel_number: channel.channel_number,
    }))
}

/// PURPOSE: Load or save `load channel`.
/// USE: Internal store helpers during multi-step transactions.
async fn load_channel(
    transaction: &impl ConnectionTrait,
    configuration: &RelayChannelConfiguration,
) -> Result<ChannelContext, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_configuration_store::apply", "load_channel");
    let channel = relay_channel::Entity::find_by_id(configuration.channel_id)
        .filter(relay_channel::Column::RelayId.eq(configuration.relay_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
        .one(transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::NotFound(format!(
                "Relay channel {} not found for relay {}",
                configuration.channel_id, configuration.relay_id
            ))
        })?;
    let relay_row = relay::Entity::find_by_id(configuration.relay_id)
        .filter(relay::Column::DeletedAt.is_null())
        .one(transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::NotFound(format!(
                "Relay channel {} not found for relay {}",
                configuration.channel_id, configuration.relay_id
            ))
        })?;
    let controller = device_controller::Entity::find()
        .filter(device_controller::Column::DeviceControllerId.eq(&relay_row.device_controller_id))
        .filter(device_controller::Column::DeletedAt.is_null())
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
        .one(transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::NotFound(format!(
                "Relay channel {} not found for relay {}",
                configuration.channel_id, configuration.relay_id
            ))
        })?;
    let channel_state = match channel.state {
        Some(relay_channel::ChannelState::On) => "on",
        _ => "off",
    }
    .to_owned();
    let mappings = if controller.mappings.is_null() {
        serde_json::json!({})
    } else {
        controller.mappings
    };
    Ok(ChannelContext {
        controller_address: controller.ip_address,
        mappings,
        relay_serial: relay_row.serial_number,
        current_device_id: channel.device_id,
        channel_number: channel.channel_number,
        channel_state,
    })
}

/// PURPOSE: Load or save `load device`.
/// USE: Internal store helpers during multi-step transactions.
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

/// PURPOSE: Load or save `save mappings`.
/// USE: Internal store helpers during multi-step transactions.
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

/// PURPOSE: Serialize `load channel json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
async fn load_channel_json(
    transaction: &impl ConnectionTrait,
    channel_id: Uuid,
) -> Result<Value, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::relay_configuration_store::apply",
        "load_channel_json",
    );
    relay_channel::Entity::find_by_id(channel_id)
        .one(transaction)
        .await?
        .map(|row| relay_channel_json(&row))
        .ok_or_else(|| DeviceRepositoryError::NotFound("Relay channel not found".to_owned()))
}

/// PURPOSE: Normalize `normalized device id` (MAC/keys) for stable lookups.
/// USE: Registration, mapping, and heartbeat correlation paths.
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

/// PURPOSE: Implement `device_mac` domain/persistence logic.
/// USE: Called as `device_mac` from handlers, repository impls, or sibling helpers.
fn device_mac(device: &DeviceContext) -> String {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_configuration_store::apply", "device_mac");
    device
        .mac_address
        .clone()
        .filter(|mac| !mac.trim().is_empty())
        .unwrap_or_else(|| device.device_id.clone())
}
