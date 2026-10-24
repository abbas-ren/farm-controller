use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait,
    FromQueryResult, QueryFilter, Statement, TransactionTrait, sea_query::Expr,
};
use uuid::Uuid;

use crate::persistence::entity::{pending_relay_config, relay_channel};

use super::super::{RelayConfirmationResponse, error::DeviceRepositoryError};

#[derive(Debug, FromQueryResult)]
struct PendingRow {
    pending_id: Uuid,
    channel_id: Uuid,
    channel_number: i32,
    relay_id: Uuid,
    controller_id: String,
    device_id: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct CountRow {
    count: i64,
}

pub(crate) async fn confirm(
    orm: &DatabaseConnection,
    mac: &str,
    succeeded: bool,
) -> Result<RelayConfirmationResponse, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::relay_configuration_store::confirmation", "confirm");
    let normalized_mac = mac.trim().to_lowercase();
    if normalized_mac.is_empty() {
        return Err(DeviceRepositoryError::Validation(
            "Missing or invalid mac in request body".to_owned(),
        ));
    }
    let transaction = orm.begin().await?;
    let pending = PendingRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT pending.id AS pending_id, pending."relayChannelId" AS channel_id,
                  pending."channelNo" AS channel_number,
                  relay.id AS relay_id, relay."deviceControllerId" AS controller_id,
                  channel."deviceId" AS device_id
           FROM pending_relay_configs pending
           JOIN relay_channels channel ON channel.id = pending."relayChannelId"
           JOIN relays relay ON relay.id = channel."relayId"
           WHERE lower(pending."deviceMac") = $1 AND pending.status::text = 'pending'
           ORDER BY pending."createdAt" ASC LIMIT 1 FOR UPDATE"#,
        [normalized_mac.clone().into()],
    ))
    .one(&transaction)
    .await?;
    let Some(PendingRow {
        pending_id,
        channel_id,
        channel_number,
        relay_id,
        controller_id,
        device_id,
    }) = pending
    else {
        transaction.commit().await?;
        return Ok(RelayConfirmationResponse {
            confirmed: false,
            message: format!("No pending configuration found for MAC {normalized_mac}"),
            controller_id: None,
            relay_id: None,
            device_id: None,
            configuration_complete: false,
        });
    };
    let now = chrono::Utc::now();
    if !succeeded {
        pending_relay_config::Entity::update_many()
            .col_expr(
                pending_relay_config::Column::Status,
                Expr::value(pending_relay_config::PendingRelayConfigStatus::Failed),
            )
            .col_expr(pending_relay_config::Column::UpdatedAt, Expr::value(now))
            .filter(pending_relay_config::Column::Id.eq(pending_id))
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        return Ok(RelayConfirmationResponse {
            confirmed: false,
            message: format!("Configuration failed for MAC {normalized_mac}"),
            controller_id: Some(controller_id),
            relay_id: Some(relay_id),
            device_id,
            configuration_complete: false,
        });
    }
    pending_relay_config::Entity::update_many()
        .col_expr(
            pending_relay_config::Column::Status,
            Expr::value(pending_relay_config::PendingRelayConfigStatus::Confirmed),
        )
        .col_expr(pending_relay_config::Column::UpdatedAt, Expr::value(now))
        .filter(pending_relay_config::Column::Id.eq(pending_id))
        .exec(&transaction)
        .await?;
    relay_channel::Entity::update_many()
        .col_expr(
            relay_channel::Column::State,
            Expr::value(Some(relay_channel::ChannelState::On)),
        )
        .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
        .filter(relay_channel::Column::Id.eq(channel_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .exec(&transaction)
        .await?;
    // Subquery update for device power kept as raw SQL.
    transaction
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"UPDATE devices SET power = 'on', "updatedAt" = now()
               WHERE "deviceId" = (
                 SELECT "deviceId" FROM relay_channels WHERE id = $1
               ) AND "deletedAt" IS NULL"#,
            [channel_id.into()],
        ))
        .await?;
    let remaining = CountRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT count(*) AS count
           FROM pending_relay_configs pending
           JOIN relay_channels channel ON channel.id = pending."relayChannelId"
           WHERE channel."relayId" = $1 AND pending.status::text = 'pending'"#,
        [relay_id.into()],
    ))
    .one(&transaction)
    .await?
    .map(|row| row.count)
    .unwrap_or(0);
    transaction.commit().await?;
    tracing::info!(%normalized_mac, channel_number, "relay configuration confirmed");
    Ok(RelayConfirmationResponse {
        confirmed: true,
        message: format!("Configuration confirmed for MAC {normalized_mac}"),
        controller_id: Some(controller_id),
        relay_id: Some(relay_id),
        device_id,
        configuration_complete: remaining == 0,
    })
}
