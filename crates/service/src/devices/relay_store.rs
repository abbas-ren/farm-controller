//! Modern relay inventory, power targets, and channel state persistence.
//!
//! Backs power / relay HTTP handlers and `PowerRelayRepository` for Gen4/Gen5
//! controllers. Ports Node `RelayService` power and channel listing paths.

use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, TransactionTrait,
    sea_query::{Expr, extension::postgres::PgExpr},
};
use uuid::Uuid;

use crate::persistence::entity::{device, device_controller, pending_relay_config, relay, relay_channel};

use super::{
    AvailableRelayDevicesQuery,
    constants::MAX_PAGE_SIZE,
    error::DeviceRepositoryError,
    postgres::json_models::{
        available_relay_device_json, mapping_power_port, normalize_mapping_lookup_key, relay_channel_json,
        relay_channel_with_device_json, relay_json,
    },
    repository_types::{DevicePowerTarget, RelayIdentityTarget, RelayStateTarget},
};

/// PURPOSE: Parse API/DB string into the `parse channel state` domain enum/value.
/// USE: Store ingest paths converting wire strings to SeaORM enums.
fn parse_channel_state(state: &str) -> Result<relay_channel::ChannelState, DeviceRepositoryError> {
    match state {
        "on" => Ok(relay_channel::ChannelState::On),
        "off" => Ok(relay_channel::ChannelState::Off),
        other => Err(DeviceRepositoryError::Validation(format!(
            "Invalid relay channel state: {other}"
        ))),
    }
}

/// PURPOSE: Parse API/DB string into the `parse device power` domain enum/value.
/// USE: Store ingest paths converting wire strings to SeaORM enums.
fn parse_device_power(state: &str) -> Result<device::DevicePower, DeviceRepositoryError> {
    match state {
        "on" => Ok(device::DevicePower::On),
        "off" => Ok(device::DevicePower::Off),
        other => Err(DeviceRepositoryError::Validation(format!(
            "Invalid device power state: {other}"
        ))),
    }
}

/// PURPOSE: Implement `power_text` domain/persistence logic.
/// USE: Called as `power_text` from handlers, repository impls, or sibling helpers.
fn power_text(power: Option<&device::DevicePower>) -> Option<String> {
    power.map(|p| match p {
        device::DevicePower::On => "on".to_owned(),
        device::DevicePower::Off => "off".to_owned(),
    })
}

/// PURPOSE: Implement `controller_generation` domain/persistence logic.
/// USE: Called as `controller_generation` from handlers, repository impls, or sibling helpers.
fn controller_generation(device_family: Option<&str>) -> Option<String> {
    let normalized = device_family?
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if str::contains(&normalized, "gen3") {
        Some("gen3".to_owned())
    } else if str::contains(&normalized, "gen4") {
        Some("gen4".to_owned())
    } else {
        None
    }
}

/// PURPOSE: Resolve Gen4 relay or Gen5 mapping power target for a device.
/// USE: Power on/off handlers before outbound edge commands.
pub(crate) async fn device_power_target(
    orm: &DatabaseConnection,
    device_id: &str,
) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "device_power_target");
    let Some(device_row) = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .one(orm)
        .await?
    else {
        return Ok(None);
    };

    let channel_row = relay_channel::Entity::find()
        .filter(relay_channel::Column::DeviceId.eq(device_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .find_also_related(relay::Entity)
        .order_by_desc(relay_channel::Column::UpdatedAt)
        .one(orm)
        .await?;

    let (channel_id, channel_number, relay_serial, relay_controller_ip) =
        match channel_row {
            Some((channel, Some(relay_row))) if relay_row.deleted_at.is_none() => {
                let controller = device_controller::Entity::find()
                    .filter(
                        device_controller::Column::DeviceControllerId
                            .eq(&relay_row.device_controller_id),
                    )
                    .one(orm)
                    .await?;
                (
                    Some(channel.id),
                    Some(channel.channel_number),
                    Some(relay_row.serial_number),
                    controller.map(|c| c.ip_address),
                )
            }
            _ => (None, None, None, None),
        };

    let mapping_mac = device_row
        .mac_address
        .as_deref()
        .filter(|mac| !mac.is_empty())
        .unwrap_or(device_row.device_id.as_str());
    let normalized_mac = normalize_mapping_lookup_key(mapping_mac);

    let mut controllers = device_controller::Entity::find()
        .filter(device_controller::Column::State.eq(device_controller::DeviceControllerState::Active))
        .filter(
            device_controller::Column::Status.eq(device_controller::DeviceControllerStatus::Approved),
        )
        .filter(device_controller::Column::DeletedAt.is_null())
        .all(orm)
        .await?;
    controllers.sort_by(|a, b| {
        let a_preferred = Some(a.device_controller_id.as_str()) == device_row.controller_id.as_deref();
        let b_preferred = Some(b.device_controller_id.as_str()) == device_row.controller_id.as_deref();
        b_preferred
            .cmp(&a_preferred)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });

    let mapping = controllers.into_iter().find_map(|controller| {
        let power_port = mapping_power_port(&controller.mappings, &normalized_mac)?;
        // Exact-key presence check mirrors SQL `mappings ? normalized_key` after fuzzy resolve.
        Some((controller.ip_address, power_port))
    });

    let (mapping_ip, power_port) = match mapping {
        Some((ip, port)) => (Some(ip), Some(port)),
        None => (None, None),
    };

    Ok(Some(DevicePowerTarget {
        device_family: device_row.device_family,
        current_power: power_text(device_row.power.as_ref()),
        controller_ip: relay_controller_ip.or(mapping_ip),
        power_port,
        relay_serial,
        channel_id,
        channel_number,
    }))
}

/// PURPOSE: Persist device power state (and relay channel state when applicable).
/// USE: After the edge acknowledges a power command.
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
                crate::persistence::pg_enum::pg_enum(&channel_state),
            )
            .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
            .filter(relay_channel::Column::Id.eq(channel_id))
            .filter(relay_channel::Column::DeletedAt.is_null())
            .exec(&transaction)
            .await?;
    }
    let updated = device::Entity::update_many()
        .col_expr(
            device::Column::Power,
            crate::persistence::pg_enum::pg_enum(&power),
        )
        .col_expr(device::Column::UpdatedAt, Expr::value(now))
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .exec(&transaction)
        .await?
        .rows_affected;
    let channel = if let Some(channel_id) = channel_id {
        relay_channel::Entity::find_by_id(channel_id)
            .one(&transaction)
            .await?
            .map(|row| relay_channel_json(&row))
    } else {
        None
    };
    transaction.commit().await?;
    Ok((updated > 0).then(|| {
        channel.unwrap_or_else(|| serde_json::json!({"deviceId": device_id, "state": state}))
    }))
}

/// PURPOSE: Implement `relays_for_controller` domain/persistence logic.
/// USE: Called as `relays_for_controller` from handlers, repository impls, or sibling helpers.
pub(crate) async fn relays_for_controller(
    orm: &DatabaseConnection,
    controller_id: &str,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "relays_for_controller");
    let relays = relay::Entity::find()
        .filter(relay::Column::DeviceControllerId.eq(controller_id))
        .filter(relay::Column::DeletedAt.is_null())
        .order_by_asc(relay::Column::CreatedAt)
        .all(orm)
        .await?;
    let mut values = Vec::with_capacity(relays.len());
    for relay_model in relays {
        let channels = relay_channel::Entity::find()
            .filter(relay_channel::Column::RelayId.eq(relay_model.id))
            .filter(relay_channel::Column::DeletedAt.is_null())
            .order_by_asc(relay_channel::Column::ChannelNumber)
            .all(orm)
            .await?;
        let mut channel_values = Vec::with_capacity(channels.len());
        for channel in channels {
            let device = if let Some(device_id) = channel.device_id.as_deref() {
                device::Entity::find()
                    .filter(device::Column::DeviceId.eq(device_id))
                    .one(orm)
                    .await?
            } else {
                None
            };
            channel_values.push(relay_channel_with_device_json(&channel, device.as_ref()));
        }
        let mut relay_value = relay_json(&relay_model);
        relay_value["relayChannels"] = serde_json::Value::Array(channel_values);
        values.push(relay_value);
    }
    Ok(values)
}

/// PURPOSE: Implement `channels_for_relay` domain/persistence logic.
/// USE: Called as `channels_for_relay` from handlers, repository impls, or sibling helpers.
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
    let channels = relay_channel::Entity::find()
        .filter(relay_channel::Column::RelayId.eq(relay_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .order_by_asc(relay_channel::Column::ChannelNumber)
        .all(orm)
        .await?;
    let mut values = Vec::with_capacity(channels.len());
    for channel in channels {
        let device = if let Some(device_id) = channel.device_id.as_deref() {
            device::Entity::find()
                .filter(device::Column::DeviceId.eq(device_id))
                .one(orm)
                .await?
        } else {
            None
        };
        values.push(relay_channel_with_device_json(&channel, device.as_ref()));
    }
    Ok(Some(values))
}

/// PURPOSE: Implement `available_devices` domain/persistence logic.
/// USE: Called as `available_devices` from handlers, repository impls, or sibling helpers.
pub(crate) async fn available_devices(
    orm: &DatabaseConnection,
    query: &AvailableRelayDevicesQuery,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "available_devices");
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(10).clamp(1, MAX_PAGE_SIZE);
    let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);

    let generation = if let Some(relay_id) = query.relay_id {
        let relay_row = relay::Entity::find_by_id(relay_id)
            .filter(relay::Column::DeletedAt.is_null())
            .one(orm)
            .await?;
        match relay_row {
            Some(relay_row) => {
                let controller = device_controller::Entity::find()
                    .filter(
                        device_controller::Column::DeviceControllerId
                            .eq(&relay_row.device_controller_id),
                    )
                    .one(orm)
                    .await?;
                controller_generation(controller.and_then(|c| c.device_family).as_deref())
            }
            None => None,
        }
    } else {
        None
    };

    // When relay_id is None: any mapped device is claimed.
    // When Some: only mappings on *other* relays claim the device.
    let claimed_device_ids: Vec<String> = {
        let mut select = relay_channel::Entity::find()
            .filter(relay_channel::Column::DeletedAt.is_null())
            .filter(relay_channel::Column::DeviceId.is_not_null());
        if let Some(relay_id) = query.relay_id {
            select = select.filter(relay_channel::Column::RelayId.ne(relay_id));
        }
        select
            .all(orm)
            .await?
            .into_iter()
            .filter_map(|channel| channel.device_id)
            .collect()
    };

    let mut base = device::Entity::find().filter(device::Column::DeletedAt.is_null());
    if !claimed_device_ids.is_empty() {
        base = base.filter(device::Column::DeviceId.is_not_in(claimed_device_ids));
    }
    if let Some(generation) = generation.as_deref() {
        let pattern = format!("%{generation}%");
        base = base.filter(
            Condition::any()
                .add(Expr::col(device::Column::DeviceFamily).ilike(&pattern))
                .add(Expr::col(device::Column::DeviceType).ilike(&pattern)),
        );
    }

    let total_count = base.clone().count(orm).await? as i64;
    let devices = base
        .order_by_asc(device::Column::DeviceId)
        .limit(u64::from(limit))
        .offset(offset as u64)
        .all(orm)
        .await?
        .iter()
        .map(available_relay_device_json)
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "devices": devices,
        "pagination": {
            "page": page, "limit": limit, "totalCount": total_count,
            "totalPages": u64::try_from(total_count).unwrap_or(0).div_ceil(u64::from(limit)),
        }
    }))
}

/// PURPOSE: Resolve edge/controller targets for `state target`.
/// USE: Handlers/workers before outbound edge HTTP.
pub(crate) async fn state_target(
    orm: &DatabaseConnection,
    device_id: &str,
) -> Result<Option<RelayStateTarget>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_store", "state_target");
    let Some((channel, Some(relay_row))) = relay_channel::Entity::find()
        .filter(relay_channel::Column::DeviceId.eq(device_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .find_also_related(relay::Entity)
        .order_by_desc(relay_channel::Column::UpdatedAt)
        .one(orm)
        .await?
    else {
        return Ok(None);
    };
    if relay_row.deleted_at.is_some() {
        return Ok(None);
    }
    let Some(controller) = device_controller::Entity::find()
        .filter(device_controller::Column::DeviceControllerId.eq(&relay_row.device_controller_id))
        .filter(device_controller::Column::DeletedAt.is_null())
        .one(orm)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(RelayStateTarget {
        controller_ip: controller.ip_address,
        relay_serial: relay_row.serial_number,
        channel_id: channel.id,
        channel_number: channel.channel_number,
    }))
}

/// PURPOSE: Mutate persisted state for `update state`.
/// USE: Update handlers and workers after side effects succeed.
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
            crate::persistence::pg_enum::pg_enum(&channel_state),
        )
        .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
        .filter(relay_channel::Column::Id.eq(channel_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .exec(orm)
        .await?;
    Ok(())
}

/// PURPOSE: Resolve edge/controller targets for `identity target`.
/// USE: Handlers/workers before outbound edge HTTP.
pub(crate) async fn identity_target(
    orm: &DatabaseConnection,
    relay_id: Uuid,
) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_store", "identity_target");
    let Some(relay_row) = relay::Entity::find_by_id(relay_id)
        .filter(relay::Column::DeletedAt.is_null())
        .one(orm)
        .await?
    else {
        return Ok(None);
    };
    let Some(controller) = device_controller::Entity::find()
        .filter(device_controller::Column::DeviceControllerId.eq(&relay_row.device_controller_id))
        .filter(device_controller::Column::DeletedAt.is_null())
        .one(orm)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(RelayIdentityTarget {
        controller_id: relay_row.device_controller_id,
        controller_ip: controller.ip_address,
        current_serial: relay_row.serial_number,
    }))
}

/// PURPOSE: Mutate persisted state for `update identity`.
/// USE: Update handlers and workers after side effects succeed.
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
    let duplicate = relay::Entity::find()
        .filter(relay::Column::SerialNumber.eq(serial_number))
        .filter(relay::Column::Id.ne(relay_id))
        .filter(relay::Column::DeletedAt.is_null())
        .one(&transaction)
        .await?
        .is_some();
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
