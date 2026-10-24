use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait,
    sea_query::{Expr, LockType},
};
use uuid::Uuid;

use crate::persistence::entity::{device, device_controller, relay, relay_channel};

use super::{
    LegacyRelayChannelListQuery, LegacyRelayConfiguration, LegacyRelayConflictQuery,
    LegacyRelayListQuery, RelayConflictState,
    error::DeviceRepositoryError,
    postgres::json_models::{relay_channel_json, relay_json},
    repository_types::{RelayConfigurationResult, RelayControllerAction},
};

pub(crate) async fn list_relays(
    orm: &DatabaseConnection,
    query: &LegacyRelayListQuery,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "list_relays");
    let mut select = relay::Entity::find().filter(relay::Column::DeletedAt.is_null());
    if let Some(controller_id) = query.controller_id.as_deref() {
        select = select.filter(relay::Column::DeviceControllerId.eq(controller_id));
    }
    Ok(select
        .order_by_asc(relay::Column::CreatedAt)
        .all(orm)
        .await?
        .iter()
        .map(relay_json)
        .collect())
}

pub(crate) async fn list_channels(
    orm: &DatabaseConnection,
    query: &LegacyRelayChannelListQuery,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "list_channels");
    let mut select = relay_channel::Entity::find().filter(relay_channel::Column::DeletedAt.is_null());
    if let Some(device_id) = query.device_id.as_deref() {
        select = select.filter(relay_channel::Column::DeviceId.eq(device_id));
    }
    if let Some(relay_id) = query.relay_id {
        select = select.filter(relay_channel::Column::RelayId.eq(relay_id));
    }
    Ok(select
        .order_by_asc(relay_channel::Column::CreatedAt)
        .all(orm)
        .await?
        .iter()
        .map(relay_channel_json)
        .collect())
}

pub(crate) async fn relay_by_id(
    orm: &DatabaseConnection,
    relay_id: Uuid,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "relay_by_id");
    Ok(relay::Entity::find_by_id(relay_id)
        .filter(relay::Column::DeletedAt.is_null())
        .one(orm)
        .await?
        .map(|row| relay_json(&row)))
}

pub(crate) async fn channel_by_id(
    orm: &DatabaseConnection,
    channel_id: Uuid,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "channel_by_id");
    Ok(relay_channel::Entity::find_by_id(channel_id)
        .filter(relay_channel::Column::DeletedAt.is_null())
        .one(orm)
        .await?
        .map(|row| relay_channel_json(&row)))
}

pub(crate) async fn delete_relay(
    orm: &DatabaseConnection,
    relay_id: Uuid,
) -> Result<bool, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "delete_relay");
    let now = chrono::Utc::now();
    let result = relay::Entity::update_many()
        .col_expr(relay::Column::DeletedAt, Expr::value(Some(now)))
        .col_expr(relay::Column::UpdatedAt, Expr::value(now))
        .filter(relay::Column::Id.eq(relay_id))
        .filter(relay::Column::DeletedAt.is_null())
        .exec(orm)
        .await?;
    Ok(result.rows_affected > 0)
}

pub(crate) async fn delete_channel(
    orm: &DatabaseConnection,
    channel_id: Uuid,
) -> Result<bool, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "delete_channel");
    let now = chrono::Utc::now();
    let result = relay_channel::Entity::update_many()
        .col_expr(relay_channel::Column::DeletedAt, Expr::value(Some(now)))
        .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
        .filter(relay_channel::Column::Id.eq(channel_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .exec(orm)
        .await?;
    Ok(result.rows_affected > 0)
}

pub(crate) async fn configure(
    orm: &DatabaseConnection,
    request: &LegacyRelayConfiguration,
) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "configure");
    let channel_id = request.id.ok_or_else(|| {
        DeviceRepositoryError::Validation("Relay channel id is required".to_owned())
    })?;
    let device_id = request
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DeviceRepositoryError::Validation("Device id is required".to_owned()))?;
    let transaction = orm.begin().await?;
    let channel = relay_channel::Entity::find_by_id(channel_id)
        .filter(relay_channel::Column::DeletedAt.is_null())
        .lock(LockType::Update)
        .one(&transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::NotFound(
                "Device, relay channel, or controller not found".to_owned(),
            )
        })?;
    let relay_row = relay::Entity::find_by_id(channel.relay_id)
        .filter(relay::Column::DeletedAt.is_null())
        .one(&transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::NotFound(
                "Device, relay channel, or controller not found".to_owned(),
            )
        })?;
    let controller = device_controller::Entity::find()
        .filter(device_controller::Column::DeviceControllerId.eq(&relay_row.device_controller_id))
        .filter(device_controller::Column::DeletedAt.is_null())
        .one(&transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::NotFound(
                "Device, relay channel, or controller not found".to_owned(),
            )
        })?;
    let device_row = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .one(&transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::NotFound(
                "Device, relay channel, or controller not found".to_owned(),
            )
        })?;
    let device_mac = device_row
        .mac_address
        .as_deref()
        .map(str::trim)
        .filter(|mac| !mac.is_empty())
        .unwrap_or(device_row.device_id.as_str())
        .to_owned();
    let now = chrono::Utc::now();
    relay_channel::Entity::update_many()
        .col_expr(
            relay_channel::Column::DeviceId,
            Expr::value(Option::<String>::None),
        )
        .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
        .filter(relay_channel::Column::DeviceId.eq(device_id))
        .filter(relay_channel::Column::Id.ne(channel_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .exec(&transaction)
        .await?;
    relay_channel::Entity::update_many()
        .col_expr(
            relay_channel::Column::DeviceId,
            Expr::value(Some(device_id.to_owned())),
        )
        .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
        .filter(relay_channel::Column::Id.eq(channel_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .exec(&transaction)
        .await?;
    let channel_value = channel_json(&transaction, channel_id).await?;
    transaction.commit().await?;
    Ok(RelayConfigurationResult {
        channels: vec![channel_value],
        actions: vec![RelayControllerAction::Configure {
            controller_address: controller.ip_address,
            device_mac,
            relay_serial: relay_row.serial_number,
            channel_number: channel.channel_number,
            device_generation: device_row.device_type,
        }],
    })
}

pub(crate) async fn fresh(orm: &DatabaseConnection, relay_id: Uuid) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::legacy_relay_store", "fresh");
    let transaction = orm.begin().await?;
    let relay_row = relay::Entity::find_by_id(relay_id)
        .filter(relay::Column::DeletedAt.is_null())
        .one(&transaction)
        .await?
        .ok_or_else(|| DeviceRepositoryError::NotFound("Relay not found".to_owned()))?;
    relay::Entity::delete_many()
        .filter(relay::Column::SerialNumber.eq(&relay_row.serial_number))
        .filter(relay::Column::DeviceControllerId.ne(&relay_row.device_controller_id))
        .exec(&transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

pub(crate) async fn remap(
    orm: &DatabaseConnection,
    relay_id: Uuid,
) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::legacy_relay_store", "remap");
    let transaction = orm.begin().await?;
    let relay_row = relay::Entity::find_by_id(relay_id)
        .filter(relay::Column::DeletedAt.is_null())
        .lock(LockType::Update)
        .one(&transaction)
        .await?
        .ok_or_else(|| DeviceRepositoryError::NotFound("Relay or controller not found".to_owned()))?;
    let controller = device_controller::Entity::find()
        .filter(device_controller::Column::DeviceControllerId.eq(&relay_row.device_controller_id))
        .filter(device_controller::Column::DeletedAt.is_null())
        .one(&transaction)
        .await?
        .ok_or_else(|| DeviceRepositoryError::NotFound("Relay or controller not found".to_owned()))?;
    let previous_relay = relay::Entity::find()
        .filter(relay::Column::SerialNumber.eq(&relay_row.serial_number))
        .filter(relay::Column::Id.ne(relay_row.id))
        .filter(relay::Column::DeviceControllerId.ne(&relay_row.device_controller_id))
        .order_by_asc(relay::Column::CreatedAt)
        .select_only()
        .column(relay::Column::Id)
        .into_tuple::<Uuid>()
        .one(&transaction)
        .await?;
    if let Some(previous_relay) = previous_relay {
        let now = chrono::Utc::now();
        relay_channel::Entity::update_many()
            .col_expr(
                relay_channel::Column::DeletedAt,
                Expr::value(Option::<chrono::DateTime<chrono::Utc>>::None),
            )
            .col_expr(relay_channel::Column::RelayId, Expr::value(relay_row.id))
            .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
            .filter(relay_channel::Column::RelayId.eq(previous_relay))
            .exec(&transaction)
            .await?;
        relay::Entity::delete_by_id(previous_relay)
            .exec(&transaction)
            .await?;
    }
    let channels = relay_channel::Entity::find()
        .filter(relay_channel::Column::RelayId.eq(relay_row.id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .order_by_asc(relay_channel::Column::ChannelNumber)
        .all(&transaction)
        .await?;
    let mut channel_values = Vec::new();
    let mut actions = Vec::new();
    for channel in channels {
        let Some(device_id) = channel.device_id.as_deref() else {
            continue;
        };
        let Some(device_row) = device::Entity::find()
            .filter(device::Column::DeviceId.eq(device_id))
            .filter(device::Column::DeletedAt.is_null())
            .one(&transaction)
            .await?
        else {
            continue;
        };
        let device_mac = device_row
            .mac_address
            .as_deref()
            .map(str::trim)
            .filter(|mac| !mac.is_empty())
            .unwrap_or(device_row.device_id.as_str())
            .to_owned();
        channel_values.push(relay_channel_json(&channel));
        actions.push(RelayControllerAction::Configure {
            controller_address: controller.ip_address.clone(),
            device_mac,
            relay_serial: relay_row.serial_number.clone(),
            channel_number: channel.channel_number,
            device_generation: device_row.device_type,
        });
    }
    transaction.commit().await?;
    Ok(RelayConfigurationResult {
        channels: channel_values,
        actions,
    })
}

pub(crate) async fn check_conflicts(
    orm: &DatabaseConnection,
    query: &LegacyRelayConflictQuery,
) -> Result<RelayConflictState, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "check_conflicts");
    let transaction = orm.begin().await?;
    let serial_number = relay::Entity::find_by_id(query.relay_id)
        .filter(relay::Column::DeletedAt.is_null())
        .one(&transaction)
        .await?
        .ok_or_else(|| DeviceRepositoryError::NotFound("Relay not found".to_owned()))?
        .serial_number;
    let device_conflict = if let Some(device_id) = query.device_id.as_deref() {
        relay_channel::Entity::find()
            .filter(relay_channel::Column::DeviceId.eq(device_id))
            .filter(relay_channel::Column::DeletedAt.is_null())
            .filter(
                sea_orm::Condition::any()
                    .add(relay_channel::Column::RelayId.ne(query.relay_id))
                    .add(relay_channel::Column::ChannelNumber.ne(query.channel_number)),
            )
            .one(&transaction)
            .await?
            .is_some()
    } else {
        false
    };
    let channel_conflict = if let Some(device_id) = query.device_id.as_deref() {
        relay_channel::Entity::find()
            .filter(relay_channel::Column::RelayId.eq(query.relay_id))
            .filter(relay_channel::Column::ChannelNumber.eq(query.channel_number))
            .filter(relay_channel::Column::DeviceId.ne(device_id))
            .filter(relay_channel::Column::DeletedAt.is_null())
            .one(&transaction)
            .await?
            .is_some()
    } else {
        false
    };
    let duplicate_relays = relay::Entity::find()
        .filter(relay::Column::SerialNumber.eq(serial_number))
        .filter(relay::Column::Id.ne(query.relay_id))
        .select_only()
        .column(relay::Column::Id)
        .into_tuple::<Uuid>()
        .all(&transaction)
        .await?;
    let relay_conflict = if duplicate_relays.is_empty() {
        false
    } else {
        let has_channels = relay_channel::Entity::find()
            .filter(relay_channel::Column::RelayId.is_in(duplicate_relays.clone()))
            .one(&transaction)
            .await?
            .is_some();
        if !has_channels {
            relay::Entity::delete_many()
                .filter(relay::Column::Id.is_in(duplicate_relays))
                .exec(&transaction)
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
    transaction: &impl ConnectionTrait,
    channel_id: Uuid,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "channel_json");
    relay_channel::Entity::find_by_id(channel_id)
        .one(transaction)
        .await?
        .map(|row| relay_channel_json(&row))
        .ok_or_else(|| DeviceRepositoryError::NotFound("Relay channel not found".to_owned()))
}
