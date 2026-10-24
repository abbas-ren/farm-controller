//! Apply relay configuration success/failure callbacks from the edge.
//!
//! Used by confirm-relay handlers and timeout workers.

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait,
    sea_query::{Expr, LockType},
};

use crate::persistence::entity::{device, pending_relay_config, relay, relay_channel};

use super::super::{RelayConfirmationResponse, error::DeviceRepositoryError};

/// PURPOSE: Record relay configuration success/failure from the edge callback.
/// USE: Confirm-relay handler and RelayConfigTimeoutService equivalent.
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
    let pending = pending_relay_config::Entity::find()
        .filter(Expr::cust_with_values(
            r#"lower("deviceMac") = $1"#,
            [normalized_mac.clone()],
        ))
        .filter(
            pending_relay_config::Column::Status
                .eq(pending_relay_config::PendingRelayConfigStatus::Pending),
        )
        .order_by_asc(pending_relay_config::Column::CreatedAt)
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
        .one(&transaction)
        .await?;
    let Some(pending) = pending else {
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
    let channel = relay_channel::Entity::find_by_id(pending.relay_channel_id)
        .one(&transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::Internal("pending relay channel missing".to_owned())
        })?;
    let relay_row = relay::Entity::find_by_id(channel.relay_id)
        .one(&transaction)
        .await?
        .ok_or_else(|| DeviceRepositoryError::Internal("pending relay missing".to_owned()))?;
    let pending_id = pending.id;
    let channel_id = pending.relay_channel_id;
    let channel_number = pending.channel_no;
    let relay_id = relay_row.id;
    let controller_id = relay_row.device_controller_id.clone();
    let device_id = channel.device_id.clone();

    let now = chrono::Utc::now();
    if !succeeded {
        pending_relay_config::Entity::update_many()
            .col_expr(
                pending_relay_config::Column::Status,
                crate::persistence::pg_enum::pg_enum(&pending_relay_config::PendingRelayConfigStatus::Failed),
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
            crate::persistence::pg_enum::pg_enum(&pending_relay_config::PendingRelayConfigStatus::Confirmed),
        )
        .col_expr(pending_relay_config::Column::UpdatedAt, Expr::value(now))
        .filter(pending_relay_config::Column::Id.eq(pending_id))
        .exec(&transaction)
        .await?;
    relay_channel::Entity::update_many()
        .col_expr(
            relay_channel::Column::State,
            crate::persistence::pg_enum::pg_enum(&relay_channel::ChannelState::On),
        )
        .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
        .filter(relay_channel::Column::Id.eq(channel_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .exec(&transaction)
        .await?;
    if let Some(mapped_device_id) = channel.device_id.as_deref() {
        device::Entity::update_many()
            .col_expr(
                device::Column::Power,
                crate::persistence::pg_enum::pg_enum(&device::DevicePower::On),
            )
            .col_expr(device::Column::UpdatedAt, Expr::value(now))
            .filter(device::Column::DeviceId.eq(mapped_device_id))
            .filter(device::Column::DeletedAt.is_null())
            .exec(&transaction)
            .await?;
    }
    let channel_ids_on_relay: Vec<uuid::Uuid> = relay_channel::Entity::find()
        .filter(relay_channel::Column::RelayId.eq(relay_id))
        .select_only()
        .column(relay_channel::Column::Id)
        .into_tuple::<uuid::Uuid>()
        .all(&transaction)
        .await?;
    let remaining = if channel_ids_on_relay.is_empty() {
        0
    } else {
        pending_relay_config::Entity::find()
            .filter(pending_relay_config::Column::RelayChannelId.is_in(channel_ids_on_relay))
            .filter(
                pending_relay_config::Column::Status
                    .eq(pending_relay_config::PendingRelayConfigStatus::Pending),
            )
            .count(&transaction)
            .await? as i64
    };
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
