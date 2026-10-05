use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{
    ControllerRegistration, DeviceController, Relay, RelayRegistration,
    error::DeviceRepositoryError,
    repository_types::RegistrationResult,
    validation::{normalized_controller_id, relay_channels, validate_registration},
};

#[derive(sqlx::FromRow)]
struct ControllerRow {
    device_controller_id: String,
    mac_address: String,
    ip_address: String,
    name: Option<String>,
    device_family: Option<String>,
    status: String,
    state: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    created: bool,
}

pub(crate) async fn register_controller(
    pool: &PgPool,
    registration: &ControllerRegistration,
) -> Result<RegistrationResult, DeviceRepositoryError> {
    validate_registration(registration)?;
    let controller_id = normalized_controller_id(&registration.mac_address)?;
    let mut transaction = pool.begin().await?;
    let row = sqlx::query_as::<_, ControllerRow>(
        r#"
        INSERT INTO device_controllers (
            "deviceControllerId", "macAddress", "ipAddress", "deviceFamily",
            "createdAt", "updatedAt"
        ) VALUES ($1, $2, $3, $4, now(), now())
        ON CONFLICT ("deviceControllerId") DO UPDATE SET
            "macAddress" = EXCLUDED."macAddress",
            "ipAddress" = EXCLUDED."ipAddress",
            "deviceFamily" = EXCLUDED."deviceFamily",
            state = 'active',
            "updatedAt" = now()
        RETURNING
            "deviceControllerId" AS device_controller_id,
            "macAddress" AS mac_address,
            "ipAddress" AS ip_address,
            name,
            "deviceFamily" AS device_family,
            status::text,
            state::text,
            "createdAt" AS created_at,
            "updatedAt" AS updated_at,
            (xmax = 0) AS created
        "#,
    )
    .bind(&controller_id)
    .bind(registration.mac_address.trim())
    .bind(registration.ip_address.trim())
    .bind(registration.device_family.as_deref())
    .fetch_one(&mut *transaction)
    .await?;

    sync_relay_inventory(
        &mut transaction,
        &controller_id,
        &registration.relays,
        row.created,
    )
    .await?;
    if row.created {
        sqlx::query(
            r#"INSERT INTO alerts
                  (id, user_id, title, message, type, status, is_read,
                   device_controller_id, data, created_at, updated_at)
               VALUES (gen_random_uuid(), 'ADMIN', 'New Device controller alert',
                       'New device controller onboarded: ' || $1,
                       'info', 'unread', false, $1,
                       jsonb_build_object('deviceControllerId', $1), now(), now())"#,
        )
        .bind(&controller_id)
        .execute(&mut *transaction)
        .await?;
    }
    let relays = load_relays(&mut transaction, &controller_id).await?;
    transaction.commit().await?;

    Ok(RegistrationResult {
        created: row.created,
        controller: DeviceController {
            device_controller_id: row.device_controller_id,
            mac_address: row.mac_address,
            ip_address: row.ip_address,
            name: row.name,
            device_family: row.device_family,
            status: row.status,
            state: row.state,
            created_at: row.created_at,
            updated_at: row.updated_at,
            relays,
        },
    })
}

async fn sync_relay_inventory(
    transaction: &mut Transaction<'_, Postgres>,
    controller_id: &str,
    relays: &[RelayRegistration],
    controller_created: bool,
) -> Result<(), DeviceRepositoryError> {
    if controller_created {
        for relay in relays {
            let serial_number = relay.serial_number.trim();
            validate_relay_serial(serial_number)?;
            let existing_ids = sqlx::query_scalar::<_, Uuid>(
                r#"SELECT id FROM relays
                   WHERE "serialNumber" = $1 AND "deletedAt" IS NULL
                   FOR UPDATE"#,
            )
            .bind(serial_number)
            .fetch_all(&mut **transaction)
            .await?;
            soft_delete_relays(transaction, &existing_ids).await?;
            let relay_id = create_relay(transaction, controller_id, serial_number).await?;
            upsert_reported_channels(transaction, relay_id, relay).await?;
        }
        return Ok(());
    }

    let reported_serials = relays
        .iter()
        .map(|relay| relay.serial_number.trim().to_owned())
        .collect::<Vec<_>>();
    let extra_ids = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM relays
           WHERE "deviceControllerId" = $1
             AND NOT ("serialNumber" = ANY($2::text[]))
           FOR UPDATE"#,
    )
    .bind(controller_id)
    .bind(&reported_serials)
    .fetch_all(&mut **transaction)
    .await?;
    for relay_id in extra_ids {
        retire_relay(transaction, relay_id).await?;
    }

    for relay in relays {
        let serial_number = relay.serial_number.trim();
        validate_relay_serial(serial_number)?;
        let existing = sqlx::query_as::<_, (Uuid, String)>(
            r#"SELECT id, "deviceControllerId" FROM relays
               WHERE "serialNumber" = $1
               ORDER BY "deletedAt" NULLS FIRST, "updatedAt" DESC
               LIMIT 1 FOR UPDATE"#,
        )
        .bind(serial_number)
        .fetch_optional(&mut **transaction)
        .await?;
        let relay_id = match existing {
            None => create_relay(transaction, controller_id, serial_number).await?,
            Some((relay_id, owner)) if owner == controller_id => {
                sqlx::query(
                    r#"UPDATE relays SET state = 'connected', "updatedAt" = now()
                       WHERE id = $1"#,
                )
                .bind(relay_id)
                .execute(&mut **transaction)
                .await?;
                relay_id
            }
            Some((relay_id, _)) => {
                retire_relay(transaction, relay_id).await?;
                create_relay(transaction, controller_id, serial_number).await?
            }
        };
        upsert_reported_channels(transaction, relay_id, relay).await?;
    }
    Ok(())
}

fn validate_relay_serial(serial_number: &str) -> Result<(), DeviceRepositoryError> {
    if serial_number.is_empty() {
        Err(DeviceRepositoryError::Validation(
            "relay serialNumber must not be empty".to_owned(),
        ))
    } else {
        Ok(())
    }
}

async fn create_relay(
    transaction: &mut Transaction<'_, Postgres>,
    controller_id: &str,
    serial_number: &str,
) -> Result<Uuid, DeviceRepositoryError> {
    let relay_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO relays
              (id, "serialNumber", "deviceControllerId", state, "createdAt", "updatedAt")
           VALUES ($1, $2, $3, 'connected', now(), now())"#,
    )
    .bind(relay_id)
    .bind(serial_number)
    .bind(controller_id)
    .execute(&mut **transaction)
    .await?;
    Ok(relay_id)
}

async fn retire_relay(
    transaction: &mut Transaction<'_, Postgres>,
    relay_id: Uuid,
) -> Result<(), DeviceRepositoryError> {
    let channel_count =
        sqlx::query_scalar::<_, i64>(r#"SELECT count(*) FROM relay_channels WHERE "relayId" = $1"#)
            .bind(relay_id)
            .fetch_one(&mut **transaction)
            .await?;
    if channel_count == 0 {
        sqlx::query(r#"DELETE FROM relays WHERE id = $1"#)
            .bind(relay_id)
            .execute(&mut **transaction)
            .await?;
    } else {
        soft_delete_relays(transaction, &[relay_id]).await?;
    }
    Ok(())
}

async fn soft_delete_relays(
    transaction: &mut Transaction<'_, Postgres>,
    relay_ids: &[Uuid],
) -> Result<(), DeviceRepositoryError> {
    if relay_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        r#"UPDATE relay_channels
           SET "deletedAt" = COALESCE("deletedAt", now()), "updatedAt" = now()
           WHERE "relayId" = ANY($1::uuid[])"#,
    )
    .bind(relay_ids)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        r#"UPDATE relays
           SET "deletedAt" = COALESCE("deletedAt", now()), "updatedAt" = now()
           WHERE id = ANY($1::uuid[])"#,
    )
    .bind(relay_ids)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn upsert_reported_channels(
    transaction: &mut Transaction<'_, Postgres>,
    relay_id: Uuid,
    relay: &RelayRegistration,
) -> Result<(), DeviceRepositoryError> {
    for channel in relay_channels(relay)? {
        sqlx::query(
            r#"INSERT INTO relay_channels
                  (id, "relayId", "channelNumber", "deviceId", state,
                   "createdAt", "updatedAt")
               VALUES ($1, $2, $3, $4, 'off', now(), now())
               ON CONFLICT ("relayId", "channelNumber") DO UPDATE SET
                   "deviceId" = EXCLUDED."deviceId", "deletedAt" = NULL,
                   "updatedAt" = now()"#,
        )
        .bind(channel.relay_channel_id.unwrap_or_else(Uuid::new_v4))
        .bind(relay_id)
        .bind(channel.channel_number)
        .bind(channel.device_id.as_deref())
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn load_relays(
    transaction: &mut Transaction<'_, Postgres>,
    controller_id: &str,
) -> Result<Vec<Relay>, sqlx::Error> {
    sqlx::query_as::<_, Relay>(
        r#"
        SELECT id, "serialNumber" AS serial_number, "vendorId" AS vendor_id,
               "productId" AS product_id,
               "deviceControllerId" AS device_controller_id,
               state::text, "createdAt" AS created_at, "updatedAt" AS updated_at
        FROM relays
        WHERE "deviceControllerId" = $1 AND "deletedAt" IS NULL
        ORDER BY "serialNumber"
        "#,
    )
    .bind(controller_id)
    .fetch_all(&mut **transaction)
    .await
}
