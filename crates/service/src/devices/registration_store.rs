//! Controller registration and reported relay inventory sync.
//!
//! Used when an edge controller registers or heartbeats with its relay list.
//! Ports Node `DeviceService` / `RelayService` controller registration that
//! upserts relays/channels and soft-deletes hardware no longer reported.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, LockType, NullOrdering, OnConflict, Order},
};
use uuid::Uuid;

use crate::persistence::entity::{alert, device_controller, relay, relay_channel};

use super::{
    ControllerRegistration, DeviceController, Relay, RelayRegistration,
    error::DeviceRepositoryError,
    repository_types::RegistrationResult,
    validation::{normalized_controller_id, relay_channels, validate_registration},
};

/// PURPOSE: Upsert a controller and sync reported relay inventory.
/// USE: Controller registration callback / DeviceRegistrationRepository.
pub(crate) async fn register_controller(
    orm: &DatabaseConnection,
    registration: &ControllerRegistration,
) -> Result<RegistrationResult, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::registration_store", "register_controller");
    validate_registration(registration)?;
    let controller_id = normalized_controller_id(&registration.mac_address)?;
    let transaction = orm.begin().await?;
    let now = chrono::Utc::now();
    let existing = device_controller::Entity::find_by_id(controller_id.clone())
        .one(&transaction)
        .await?;
    let (row, created) = match existing {
        Some(existing) => {
            let updated = device_controller::ActiveModel {
                device_controller_id: Set(existing.device_controller_id),
                mac_address: Set(registration.mac_address.trim().to_owned()),
                ip_address: Set(registration.ip_address.trim().to_owned()),
                device_family: Set(registration.device_family.clone()),
                state: Set(device_controller::DeviceControllerState::Active),
                updated_at: Set(now),
                ..Default::default()
            }
            .update(&transaction)
            .await?;
            (updated, false)
        }
        None => {
            let inserted = device_controller::ActiveModel {
                device_controller_id: Set(controller_id.clone()),
                mac_address: Set(registration.mac_address.trim().to_owned()),
                ip_address: Set(registration.ip_address.trim().to_owned()),
                name: Set(None),
                device_family: Set(registration.device_family.clone()),
                mappings: Set(serde_json::json!({})),
                status: Set(device_controller::DeviceControllerStatus::Approved),
                state: Set(device_controller::DeviceControllerState::Active),
                created_at: Set(now),
                updated_at: Set(now),
                deleted_at: Set(None),
            }
            .insert(&transaction)
            .await?;
            (inserted, true)
        }
    };

    sync_relay_inventory(
        &transaction,
        &controller_id,
        &registration.relays,
        created,
    )
    .await?;
    if created {
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
        created,
        controller: DeviceController {
            device_controller_id: row.device_controller_id,
            mac_address: row.mac_address,
            ip_address: row.ip_address,
            name: row.name,
            device_family: row.device_family,
            status: match row.status {
                device_controller::DeviceControllerStatus::Approved => "approved".to_owned(),
                device_controller::DeviceControllerStatus::Requested => "requested".to_owned(),
            },
            state: match row.state {
                device_controller::DeviceControllerState::NotReachable => {
                    "not-reachable".to_owned()
                }
                device_controller::DeviceControllerState::Active => "active".to_owned(),
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
            relays,
        },
    })
}

/// PURPOSE: Implement `sync_relay_inventory` domain/persistence logic.
/// USE: Called as `sync_relay_inventory` from handlers, repository impls, or sibling helpers.
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
            let existing_ids = relay::Entity::find()
                .filter(relay::Column::SerialNumber.eq(serial_number))
                .filter(relay::Column::DeletedAt.is_null())
                // Row lock: serialize concurrent updates on this controller/device/relay.
                .lock(LockType::Update)
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
    let extra_ids = relay::Entity::find()
        .filter(relay::Column::DeviceControllerId.eq(controller_id))
        .filter(relay::Column::SerialNumber.is_not_in(reported_serials))
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
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
        let existing = relay::Entity::find()
            .filter(relay::Column::SerialNumber.eq(serial_number))
            .order_by_with_nulls(relay::Column::DeletedAt, Order::Asc, NullOrdering::First)
            .order_by_desc(relay::Column::UpdatedAt)
            .limit(1)
            // Row lock: serialize concurrent updates on this controller/device/relay.
            .lock(LockType::Update)
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

/// PURPOSE: Validate `validate relay serial` input before persistence.
/// USE: Handlers/stores before insert or update.
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

/// PURPOSE: Create/persist `create relay`.
/// USE: Create handlers after validation.
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

/// PURPOSE: Implement `retire_relay` domain/persistence logic.
/// USE: Called as `retire_relay` from handlers, repository impls, or sibling helpers.
async fn retire_relay(
    transaction: &impl ConnectionTrait,
    relay_id: Uuid,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::registration_store", "retire_relay");
    let channel_count = relay_channel::Entity::find()
        .filter(relay_channel::Column::RelayId.eq(relay_id))
        .count(transaction)
        .await?;
    if channel_count == 0 {
        relay::Entity::delete_by_id(relay_id).exec(transaction).await?;
    } else {
        soft_delete_relays(transaction, &[relay_id]).await?;
    }
    Ok(())
}

/// PURPOSE: Soft-delete relays (and channels) no longer reported by the controller.
/// USE: Relay inventory sync when hardware disappears from the edge report.
async fn soft_delete_relays(
    transaction: &impl ConnectionTrait,
    relay_ids: &[Uuid],
) -> Result<(), DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::registration_store", "soft_delete_relays");
    if relay_ids.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now();
    relay_channel::Entity::update_many()
        .col_expr(
            relay_channel::Column::DeletedAt,
            Expr::cust(r#"COALESCE("deletedAt", NOW())"#),
        )
        .col_expr(relay_channel::Column::UpdatedAt, Expr::value(now))
        .filter(relay_channel::Column::RelayId.is_in(relay_ids.to_vec()))
        .exec(transaction)
        .await?;
    relay::Entity::update_many()
        .col_expr(
            relay::Column::DeletedAt,
            Expr::cust(r#"COALESCE("deletedAt", NOW())"#),
        )
        .col_expr(relay::Column::UpdatedAt, Expr::value(now))
        .filter(relay::Column::Id.is_in(relay_ids.to_vec()))
        .exec(transaction)
        .await?;
    Ok(())
}

/// PURPOSE: Implement `upsert_reported_channels` domain/persistence logic.
/// USE: Called as `upsert_reported_channels` from handlers, repository impls, or sibling helpers.
async fn upsert_reported_channels(
    transaction: &impl ConnectionTrait,
    relay_id: Uuid,
    relay_reg: &RelayRegistration,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::registration_store",
        "upsert_reported_channels",
    );
    let now = chrono::Utc::now();
    for channel in relay_channels(relay_reg)? {
        relay_channel::Entity::insert(relay_channel::ActiveModel {
            id: Set(channel.relay_channel_id.unwrap_or_else(Uuid::new_v4)),
            relay_id: Set(relay_id),
            channel_number: Set(channel.channel_number),
            device_id: Set(channel.device_id.clone()),
            state: Set(Some(relay_channel::ChannelState::Off)),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        })
        .on_conflict(
            OnConflict::columns([
                relay_channel::Column::RelayId,
                relay_channel::Column::ChannelNumber,
            ])
            .update_columns([
                relay_channel::Column::DeviceId,
                relay_channel::Column::DeletedAt,
                relay_channel::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec(transaction)
        .await?;
    }
    Ok(())
}

/// PURPOSE: Load or save `load relays`.
/// USE: Internal store helpers during multi-step transactions.
async fn load_relays(
    transaction: &impl ConnectionTrait,
    controller_id: &str,
) -> Result<Vec<Relay>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::registration_store", "load_relays");
    Ok(relay::Entity::find()
        .filter(relay::Column::DeviceControllerId.eq(controller_id))
        .filter(relay::Column::DeletedAt.is_null())
        .order_by_asc(relay::Column::SerialNumber)
        .all(transaction)
        .await?
        .into_iter()
        .map(|row| Relay {
            id: row.id,
            serial_number: row.serial_number,
            vendor_id: row.vendor_id,
            product_id: row.product_id,
            device_controller_id: row.device_controller_id,
            state: match row.state {
                relay::RelayState::Connected => "connected".to_owned(),
                relay::RelayState::Disconnected => "disconnected".to_owned(),
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect())
}
