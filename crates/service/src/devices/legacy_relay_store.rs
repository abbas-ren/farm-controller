use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait,
    FromQueryResult, QueryFilter, QueryOrder, QuerySelect, Statement, TransactionTrait,
    sea_query::Expr,
};
use uuid::Uuid;

use crate::persistence::entity::{relay, relay_channel};

use super::{
    LegacyRelayChannelListQuery, LegacyRelayConfiguration, LegacyRelayConflictQuery,
    LegacyRelayListQuery, RelayConflictState,
    error::DeviceRepositoryError,
    repository_types::{RelayConfigurationResult, RelayControllerAction},
};

#[derive(Debug, FromQueryResult)]
struct JsonRow {
    value: serde_json::Value,
}

#[derive(Debug, FromQueryResult)]
struct LegacyConfigurationContext {
    channel_id: Uuid,
    channel_number: i32,
    controller_address: String,
    device_mac: String,
    device_generation: Option<String>,
    relay_serial: String,
}

#[derive(Debug, FromQueryResult)]
struct RemapContext {
    relay_id: Uuid,
    serial_number: String,
    controller_id: String,
    controller_address: String,
}

#[derive(Debug, FromQueryResult)]
struct MappedChannel {
    channel: serde_json::Value,
    device_mac: String,
    channel_number: i32,
    device_generation: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct ExistsRow {
    exists: bool,
}

pub(crate) async fn list_relays(
    orm: &DatabaseConnection,
    query: &LegacyRelayListQuery,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "list_relays");
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(r) AS value FROM relays r
           WHERE r."deletedAt" IS NULL
             AND ($1::text IS NULL OR r."deviceControllerId" = $1)
           ORDER BY r."createdAt" ASC"#,
        [query.controller_id.clone().into()],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect())
}

pub(crate) async fn list_channels(
    orm: &DatabaseConnection,
    query: &LegacyRelayChannelListQuery,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "list_channels");
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(rc) AS value FROM relay_channels rc
           WHERE rc."deletedAt" IS NULL
             AND ($1::text IS NULL OR rc."deviceId" = $1)
             AND ($2::uuid IS NULL OR rc."relayId" = $2)
           ORDER BY rc."createdAt" ASC"#,
        [query.device_id.clone().into(), query.relay_id.into()],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect())
}

pub(crate) async fn relay_by_id(
    orm: &DatabaseConnection,
    relay_id: Uuid,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "relay_by_id");
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(r) AS value FROM relays r
           WHERE r.id = $1 AND r."deletedAt" IS NULL"#,
        [relay_id.into()],
    ))
    .one(orm)
    .await?
    .map(|row| row.value))
}

pub(crate) async fn channel_by_id(
    orm: &DatabaseConnection,
    channel_id: Uuid,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::legacy_relay_store", "channel_by_id");
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(rc) AS value FROM relay_channels rc
           WHERE rc.id = $1 AND rc."deletedAt" IS NULL"#,
        [channel_id.into()],
    ))
    .one(orm)
    .await?
    .map(|row| row.value))
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
    let context = LegacyConfigurationContext::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
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
        [channel_id.into(), device_id.into()],
    ))
    .one(&transaction)
    .await?
    .ok_or_else(|| {
        DeviceRepositoryError::NotFound("Device, relay channel, or controller not found".to_owned())
    })?;
    let now = chrono::Utc::now();
    relay_channel::Entity::update_many()
        .col_expr(relay_channel::Column::DeviceId, Expr::value(Option::<String>::None))
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
    let channel = channel_json(&transaction, context.channel_id).await?;
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
    let target = RemapContext::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT r.id AS relay_id, r."serialNumber" AS serial_number,
             r."deviceControllerId" AS controller_id,
             dc."ipAddress" AS controller_address
           FROM relays r JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId" AND dc."deletedAt" IS NULL
           WHERE r.id = $1 AND r."deletedAt" IS NULL FOR UPDATE OF r"#,
        [relay_id.into()],
    ))
    .one(&transaction)
    .await?
    .ok_or_else(|| DeviceRepositoryError::NotFound("Relay or controller not found".to_owned()))?;
    let previous_relay = relay::Entity::find()
        .filter(relay::Column::SerialNumber.eq(&target.serial_number))
        .filter(relay::Column::Id.ne(target.relay_id))
        .filter(relay::Column::DeviceControllerId.ne(&target.controller_id))
        .order_by_asc(relay::Column::CreatedAt)
        .select_only()
        .column(relay::Column::Id)
        .into_tuple::<Uuid>()
        .one(&transaction)
        .await?;
    if let Some(previous_relay) = previous_relay {
        let now = chrono::Utc::now();
        relay_channel::Entity::update_many()
            .col_expr(relay_channel::Column::DeletedAt, Expr::value(Option::<chrono::DateTime<chrono::Utc>>::None))
            .col_expr(relay_channel::Column::RelayId, Expr::value(target.relay_id))
            .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
            .filter(relay_channel::Column::RelayId.eq(previous_relay))
            .exec(&transaction)
            .await?;
        relay::Entity::delete_by_id(previous_relay)
            .exec(&transaction)
            .await?;
    }
    let mapped = MappedChannel::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(rc) AS channel,
             COALESCE(NULLIF(d."macAddress", ''), d."deviceId") AS device_mac,
             rc."channelNumber" AS channel_number, d."deviceType" AS device_generation
           FROM relay_channels rc JOIN devices d
             ON d."deviceId" = rc."deviceId" AND d."deletedAt" IS NULL
           WHERE rc."relayId" = $1 AND rc."deletedAt" IS NULL
           ORDER BY rc."channelNumber" ASC"#,
        [target.relay_id.into()],
    ))
    .all(&transaction)
    .await?;
    let channels = mapped.iter().map(|row| row.channel.clone()).collect();
    let actions = mapped
        .into_iter()
        .map(|row| RelayControllerAction::Configure {
            controller_address: target.controller_address.clone(),
            device_mac: row.device_mac,
            relay_serial: target.serial_number.clone(),
            channel_number: row.channel_number,
            device_generation: row.device_generation,
        })
        .collect();
    transaction.commit().await?;
    Ok(RelayConfigurationResult { channels, actions })
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
        ExistsRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT EXISTS(SELECT 1 FROM relay_channels
               WHERE "deviceId" = $1 AND "deletedAt" IS NULL
                 AND ("relayId" <> $2 OR "channelNumber" <> $3)) AS exists"#,
            [
                device_id.into(),
                query.relay_id.into(),
                query.channel_number.into(),
            ],
        ))
        .one(&transaction)
        .await?
        .map(|row| row.exists)
        .unwrap_or(false)
    } else {
        false
    };
    let channel_conflict = if let Some(device_id) = query.device_id.as_deref() {
        ExistsRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT EXISTS(SELECT 1 FROM relay_channels
               WHERE "relayId" = $1 AND "channelNumber" = $2
                 AND "deviceId" <> $3 AND "deletedAt" IS NULL) AS exists"#,
            [
                query.relay_id.into(),
                query.channel_number.into(),
                device_id.into(),
            ],
        ))
        .one(&transaction)
        .await?
        .map(|row| row.exists)
        .unwrap_or(false)
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
