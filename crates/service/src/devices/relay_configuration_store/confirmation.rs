use sqlx::PgPool;
use uuid::Uuid;

use super::super::{RelayConfirmationResponse, error::DeviceRepositoryError};

pub(crate) async fn confirm(
    pool: &PgPool,
    mac: &str,
    succeeded: bool,
) -> Result<RelayConfirmationResponse, DeviceRepositoryError> {
    let normalized_mac = mac.trim().to_lowercase();
    if normalized_mac.is_empty() {
        return Err(DeviceRepositoryError::Validation(
            "Missing or invalid mac in request body".to_owned(),
        ));
    }
    let mut transaction = pool.begin().await?;
    let pending = sqlx::query_as::<_, (Uuid, Uuid, i32, Uuid, String, Option<String>)>(
        r#"SELECT pending.id, pending."relayChannelId", pending."channelNo",
                  relay.id, relay."deviceControllerId", channel."deviceId"
           FROM pending_relay_configs pending
           JOIN relay_channels channel ON channel.id = pending."relayChannelId"
           JOIN relays relay ON relay.id = channel."relayId"
           WHERE lower(pending."deviceMac") = $1 AND pending.status::text = 'pending'
           ORDER BY pending."createdAt" ASC LIMIT 1 FOR UPDATE"#,
    )
    .bind(&normalized_mac)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((pending_id, channel_id, channel_number, relay_id, controller_id, device_id)) =
        pending
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
    if !succeeded {
        sqlx::query(
            r#"UPDATE pending_relay_configs SET status = 'failed', "updatedAt" = now()
               WHERE id = $1"#,
        )
        .bind(pending_id)
        .execute(&mut *transaction)
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
    sqlx::query(
        r#"UPDATE pending_relay_configs SET status = 'confirmed', "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(pending_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"UPDATE relay_channels SET state = 'on', "updatedAt" = now()
           WHERE id = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(channel_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"UPDATE devices SET power = 'on', "updatedAt" = now()
           WHERE "deviceId" = (
             SELECT "deviceId" FROM relay_channels WHERE id = $1
           ) AND "deletedAt" IS NULL"#,
    )
    .bind(channel_id)
    .execute(&mut *transaction)
    .await?;
    let remaining = sqlx::query_scalar::<_, i64>(
        r#"SELECT count(*)
           FROM pending_relay_configs pending
           JOIN relay_channels channel ON channel.id = pending."relayChannelId"
           WHERE channel."relayId" = $1 AND pending.status::text = 'pending'"#,
    )
    .bind(relay_id)
    .fetch_one(&mut *transaction)
    .await?;
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
