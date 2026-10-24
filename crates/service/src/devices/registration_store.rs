use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection,
    EntityTrait, FromQueryResult, QueryFilter, Set, Statement, TransactionTrait, sea_query::Expr,
};
use uuid::Uuid;

use crate::persistence::entity::{alert, relay};

use super::{
    ControllerRegistration, DeviceController, Relay, RelayRegistration,
    error::DeviceRepositoryError,
    repository_types::RegistrationResult,
    validation::{normalized_controller_id, relay_channels, validate_registration},
};

#[derive(Debug, FromQueryResult)]
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

#[derive(Debug, FromQueryResult)]
struct RelayOwner {
    id: Uuid,
    device_controller_id: String,
}

#[derive(Debug, FromQueryResult)]
struct CountRow {
    count: i64,
}

pub(crate) async fn register_controller(
    orm: &DatabaseConnection,
    registration: &ControllerRegistration,
) -> Result<RegistrationResult, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::registration_store", "register_controller");
    validate_registration(registration)?;
    let controller_id = normalized_controller_id(&registration.mac_address)?;
    let transaction = orm.begin().await?;
    // INSERT ... ON CONFLICT RETURNING (xmax = 0) kept as raw SQL.
    let row = ControllerRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
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
        [
            controller_id.clone().into(),
            registration.mac_address.trim().into(),
            registration.ip_address.trim().into(),
            registration.device_family.clone().into(),
        ],
    ))
    .one(&transaction)
    .await?
    .ok_or_else(|| {
        DeviceRepositoryError::Internal("controller upsert returned no row".to_owned())
    })?;

    sync_relay_inventory(
        &transaction,
        &controller_id,
        &registration.relays,
        row.created,
    )
    .await?;
    if row.created {
        let now = chrono::Utc::now();
        alert::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set("ADMIN".to_owned()),
            title: Set("New Device controller alert".to_owned()),
            message: Set(format!("New device controller onboarded: {controller_id}")),
            r#type: Set(alert::AlertType::Info),
            status: Set(alert::AlertStatus::Unread),
            is_read: Set(false),
            device_controller_id: Set(Some(controller_id.clone())),
            data: Set(Some(serde_json::json!({"deviceControllerId": controller_id}))),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&transaction)
        .await?;
    }
    let relays = load_relays(&transaction, &controller_id).await?;
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
    transaction: &impl ConnectionTrait,
    controller_id: &str,
    relays: &[RelayRegistration],
    controller_created: bool,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::registration_store",
        "sync_relay_inventory",
    );
    if controller_created {
        for relay_reg in relays {
            let serial_number = relay_reg.serial_number.trim();
            validate_relay_serial(serial_number)?;
            let existing_ids = RelayOwner::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT id, "deviceControllerId" AS device_controller_id FROM relays
                   WHERE "serialNumber" = $1 AND "deletedAt" IS NULL
                   FOR UPDATE"#,
                [serial_number.into()],
            ))
            .all(transaction)
            .await?
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
            soft_delete_relays(transaction, &existing_ids).await?;
            let relay_id = create_relay(transaction, controller_id, serial_number).await?;
            upsert_reported_channels(transaction, relay_id, relay_reg).await?;
        }
        return Ok(());
    }

    let reported_serials = relays
        .iter()
        .map(|relay_reg| relay_reg.serial_number.trim().to_owned())
        .collect::<Vec<_>>();
    let extra_ids = IdRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT id FROM relays
           WHERE "deviceControllerId" = $1
             AND NOT ("serialNumber" = ANY($2::text[]))
           FOR UPDATE"#,
        [controller_id.into(), reported_serials.into()],
    ))
    .all(transaction)
    .await?
    .into_iter()
    .map(|row| row.id)
    .collect::<Vec<_>>();
    for relay_id in extra_ids {
        retire_relay(transaction, relay_id).await?;
    }

    for relay_reg in relays {
        let serial_number = relay_reg.serial_number.trim();
        validate_relay_serial(serial_number)?;
        let existing = RelayOwner::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT id, "deviceControllerId" AS device_controller_id FROM relays
               WHERE "serialNumber" = $1
               ORDER BY "deletedAt" NULLS FIRST, "updatedAt" DESC
               LIMIT 1 FOR UPDATE"#,
            [serial_number.into()],
        ))
        .one(transaction)
        .await?;
        let relay_id = match existing {
            None => create_relay(transaction, controller_id, serial_number).await?,
            Some(row) if row.device_controller_id == controller_id => {
                let now = chrono::Utc::now();
                relay::Entity::update_many()
                    .col_expr(relay::Column::State, Expr::value(relay::RelayState::Connected))
                    .col_expr(relay::Column::UpdatedAt, Expr::value(now))
                    .filter(relay::Column::Id.eq(row.id))
                    .exec(transaction)
                    .await?;
                row.id
            }
            Some(row) => {
                retire_relay(transaction, row.id).await?;
                create_relay(transaction, controller_id, serial_number).await?
            }
        };
        upsert_reported_channels(transaction, relay_id, relay_reg).await?;
    }
    Ok(())
}

#[derive(Debug, FromQueryResult)]
struct IdRow {
    id: Uuid,
}

fn validate_relay_serial(serial_number: &str) -> Result<(), DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::registration_store", "validate_relay_serial");
    if serial_number.is_empty() {
        Err(DeviceRepositoryError::Validation(
            "relay serialNumber must not be empty".to_owned(),
        ))
    } else {
        Ok(())
    }
}

async fn create_relay(
    transaction: &impl ConnectionTrait,
    controller_id: &str,
    serial_number: &str,
) -> Result<Uuid, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::registration_store", "create_relay");
    let relay_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    relay::ActiveModel {
        id: Set(relay_id),
        serial_number: Set(serial_number.to_owned()),
        vendor_id: Set(None),
        product_id: Set(None),
        device_controller_id: Set(controller_id.to_owned()),
        state: Set(relay::RelayState::Connected),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
    }
    .insert(transaction)
    .await?;
    Ok(relay_id)
}

async fn retire_relay(
    transaction: &impl ConnectionTrait,
    relay_id: Uuid,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::registration_store", "retire_relay");
    let channel_count = CountRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT count(*) AS count FROM relay_channels WHERE "relayId" = $1"#,
        [relay_id.into()],
    ))
    .one(transaction)
    .await?
    .map(|row| row.count)
    .unwrap_or(0);
    if channel_count == 0 {
        relay::Entity::delete_by_id(relay_id).exec(transaction).await?;
    } else {
        soft_delete_relays(transaction, &[relay_id]).await?;
    }
    Ok(())
}

async fn soft_delete_relays(
    transaction: &impl ConnectionTrait,
    relay_ids: &[Uuid],
) -> Result<(), DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::registration_store", "soft_delete_relays");
    if relay_ids.is_empty() {
        return Ok(());
    }
    // COALESCE("deletedAt", now()) semantics via raw SQL.
    transaction
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"UPDATE relay_channels
               SET "deletedAt" = COALESCE("deletedAt", now()), "updatedAt" = now()
               WHERE "relayId" = ANY($1::uuid[])"#,
            [relay_ids.to_vec().into()],
        ))
        .await?;
    transaction
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"UPDATE relays
               SET "deletedAt" = COALESCE("deletedAt", now()), "updatedAt" = now()
               WHERE id = ANY($1::uuid[])"#,
            [relay_ids.to_vec().into()],
        ))
        .await?;
    Ok(())
}

async fn upsert_reported_channels(
    transaction: &impl ConnectionTrait,
    relay_id: Uuid,
    relay_reg: &RelayRegistration,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::registration_store",
        "upsert_reported_channels",
    );
    for channel in relay_channels(relay_reg)? {
        // ON CONFLICT upsert kept as raw SQL.
        transaction
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"INSERT INTO relay_channels
                      (id, "relayId", "channelNumber", "deviceId", state,
                       "createdAt", "updatedAt")
                   VALUES ($1, $2, $3, $4, 'off', now(), now())
                   ON CONFLICT ("relayId", "channelNumber") DO UPDATE SET
                       "deviceId" = EXCLUDED."deviceId", "deletedAt" = NULL,
                       "updatedAt" = now()"#,
                [
                    channel.relay_channel_id.unwrap_or_else(Uuid::new_v4).into(),
                    relay_id.into(),
                    channel.channel_number.into(),
                    channel.device_id.clone().into(),
                ],
            ))
            .await?;
    }
    Ok(())
}

async fn load_relays(
    transaction: &impl ConnectionTrait,
    controller_id: &str,
) -> Result<Vec<Relay>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::registration_store", "load_relays");
    Ok(Relay::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"
        SELECT id, "serialNumber" AS serial_number, "vendorId" AS vendor_id,
               "productId" AS product_id,
               "deviceControllerId" AS device_controller_id,
               state::text, "createdAt" AS created_at, "updatedAt" AS updated_at
        FROM relays
        WHERE "deviceControllerId" = $1 AND "deletedAt" IS NULL
        ORDER BY "serialNumber"
        "#,
        [controller_id.into()],
    ))
    .all(transaction)
    .await?)
}
