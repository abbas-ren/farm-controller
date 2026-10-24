//! Upsert device inventory rows from registration callbacks.
//!
//! Used by the device-registration HTTP handler / `DeviceRegistrationRepository`.
//! Ports Node `DeviceService.registerDevice`, including interface rewrite,
//! status/state parsing, and Gen3 controller mapping side effects.

use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, LockType, OnConflict},
};
use uuid::Uuid;

use crate::persistence::entity::{
    alert, device, device_controller, device_interface, device_state_change, device_type_folder,
    gen5_mapping_queue,
};

use super::{
    constants::GEN3_CONTROLLER_ID,
    error::DeviceRepositoryError,
    postgres::json_models::{device_json, interface_json},
    repository_types::DeviceRegistrationResult,
    types::DeviceRegistration,
    validation::{device_interfaces, validate_device_registration},
};

/// PURPOSE: Parse API/DB string into the `parse device status` domain enum/value.
/// USE: Store ingest paths converting wire strings to SeaORM enums.
fn parse_device_status(status: &str) -> Result<device::DeviceStatus, DeviceRepositoryError> {
    match status {
        "approved" => Ok(device::DeviceStatus::Approved),
        "requested" => Ok(device::DeviceStatus::Requested),
        "declined" => Ok(device::DeviceStatus::Declined),
        other => Err(DeviceRepositoryError::Validation(format!(
            "Invalid device status: {other}"
        ))),
    }
}

/// PURPOSE: Parse API/DB string into the `parse device state` domain enum/value.
/// USE: Store ingest paths converting wire strings to SeaORM enums.
fn parse_device_state(state: &str) -> Result<device::DeviceState, DeviceRepositoryError> {
    match state {
        "busy" => Ok(device::DeviceState::Busy),
        "not_reachable" => Ok(device::DeviceState::NotReachable),
        "free" => Ok(device::DeviceState::Free),
        "faulty" => Ok(device::DeviceState::Faulty),
        "unknown" => Ok(device::DeviceState::Unknown),
        other => Err(DeviceRepositoryError::Validation(format!(
            "Invalid device state: {other}"
        ))),
    }
}

/// PURPOSE: Upsert a device and rewrite its interfaces from a registration payload.
/// USE: Device registration HTTP handler after validation/MAC normalize.
pub(crate) async fn register_device(
    orm: &DatabaseConnection,
    registration: &DeviceRegistration,
) -> Result<DeviceRegistrationResult, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::device_registration_store",
        "register_device",
    );
    let device_id = validate_device_registration(registration)?;
    let transaction = orm.begin().await?;
    let existing = device::Entity::find()
        .filter(device::Column::DeviceId.eq(&device_id))
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
        .one(&transaction)
        .await?;
    let notify_addition = existing
        .as_ref()
        .is_none_or(|row| row.deleted_at.is_some());
    let notify_approval = notify_addition
        || existing.as_ref().is_some_and(|row| {
            row.deleted_at.is_none()
                && matches!(
                    row.status,
                    Some(device::DeviceStatus::Requested) | None
                )
        });

    let now = chrono::Utc::now();
    match existing.as_ref() {
        Some(row) if row.deleted_at.is_none() => {
            let status = parse_device_status(&registration.status)?;
            device::Entity::update_many()
                .col_expr(
                    device::Column::MacAddress,
                    Expr::value(Some(registration.mac_address.clone())),
                )
                .col_expr(
                    device::Column::IpAddress,
                    Expr::value(Some(registration.ip_address.clone())),
                )
                .col_expr(
                    device::Column::DeviceName,
                    Expr::value(Some(registration.device_name.clone())),
                )
                .col_expr(
                    device::Column::DeviceFamily,
                    Expr::value(registration.device_family.clone()),
                )
                .col_expr(
                    device::Column::DeviceType,
                    Expr::value(registration.device_type.clone()),
                )
                .col_expr(
                    device::Column::BuildId,
                    Expr::value(registration.build_id.clone()),
                )
                .col_expr(
                    device::Column::HeartbeatTimer,
                    Expr::value(registration.timeout as i32),
                )
                .col_expr(
                    device::Column::SoftwareVersion,
                    Expr::value(registration.software_version.clone()),
                )
                .col_expr(
                    device::Column::NfsPath,
                    Expr::value(registration.nfs_path.clone()),
                )
                .col_expr(device::Column::Status, crate::persistence::pg_enum::pg_enum(&status))
                .col_expr(device::Column::ControllerId, Expr::value(Some(String::new())))
                .col_expr(device::Column::UpdatedAt, Expr::value(now))
                .filter(device::Column::DeviceId.eq(&device_id))
                .exec(&transaction)
                .await?;
        }
        Some(_) => {
            device::Entity::update_many()
                .col_expr(
                    device::Column::MacAddress,
                    Expr::value(Some(registration.mac_address.clone())),
                )
                .col_expr(
                    device::Column::IpAddress,
                    Expr::value(Some(registration.ip_address.clone())),
                )
                .col_expr(
                    device::Column::DeviceName,
                    Expr::value(Some(registration.device_name.clone())),
                )
                .col_expr(
                    device::Column::DeviceFamily,
                    Expr::value(registration.device_family.clone()),
                )
                .col_expr(
                    device::Column::DeviceType,
                    Expr::value(registration.device_type.clone()),
                )
                .col_expr(
                    device::Column::BuildId,
                    Expr::value(registration.build_id.clone()),
                )
                .col_expr(
                    device::Column::HeartbeatTimer,
                    Expr::value(registration.timeout as i32),
                )
                .col_expr(
                    device::Column::SoftwareVersion,
                    Expr::value(registration.software_version.clone()),
                )
                .col_expr(
                    device::Column::NfsPath,
                    Expr::value(registration.nfs_path.clone()),
                )
                .col_expr(
                    device::Column::State,
                    crate::persistence::pg_enum::pg_enum(&device::DeviceState::Unknown),
                )
                .col_expr(
                    device::Column::Status,
                    crate::persistence::pg_enum::pg_enum(&device::DeviceStatus::Requested),
                )
                .col_expr(device::Column::ControllerId, Expr::value(Some(String::new())))
                .col_expr(device::Column::Upgrading, Expr::value(Some(false)))
                .col_expr(device::Column::Flashing, Expr::value(Some(false)))
                .col_expr(
                    device::Column::DeletedAt,
                    Expr::value(Option::<chrono::DateTime<chrono::Utc>>::None),
                )
                .col_expr(device::Column::StateUpdatedAt, Expr::value(Some(now)))
                .col_expr(device::Column::UpdatedAt, Expr::value(now))
                .filter(device::Column::DeviceId.eq(&device_id))
                .exec(&transaction)
                .await?;
            device_state_change::ActiveModel {
                device_id: Set(device_id.clone()),
                state: Set(device_state_change::DeviceStateChangeState::Unknown),
                changed_at: Set(now),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(&transaction)
            .await?;
        }
        None => {
            let state = parse_device_state(&registration.state)?;
            let status = parse_device_status(&registration.status)?;
            device::ActiveModel {
                device_id: Set(device_id.clone()),
                mac_address: Set(Some(registration.mac_address.clone())),
                ip_address: Set(Some(registration.ip_address.clone())),
                device_name: Set(Some(registration.device_name.clone())),
                device_family: Set(registration.device_family.clone()),
                device_type: Set(registration.device_type.clone()),
                build_id: Set(registration.build_id.clone()),
                heartbeat_timer: Set(registration.timeout as i32),
                software_version: Set(registration.software_version.clone()),
                nfs_path: Set(registration.nfs_path.clone()),
                state: Set(Some(state)),
                status: Set(Some(status)),
                controller_id: Set(Some(String::new())),
                upgrading: Set(Some(false)),
                flashing: Set(Some(false)),
                last_connected_on: Set(now),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(&transaction)
            .await?;
            alert::ActiveModel {
                id: Set(Uuid::new_v4()),
                user_id: Set("ADMIN".to_owned()),
                title: Set("New Device On board".to_owned()),
                message: Set(format!("New device onboarded: {device_id}")),
                r#type: Set(alert::AlertType::Info),
                status: Set(alert::AlertStatus::Unread),
                is_read: Set(false),
                device_id: Set(Some(device_id.clone())),
                data: Set(Some(serde_json::json!({"deviceId": device_id}))),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(&transaction)
            .await?;
        }
    }

    device_interface::Entity::delete_many()
        .filter(device_interface::Column::DeviceId.eq(&device_id))
        .exec(&transaction)
        .await?;
    for interface in device_interfaces(registration) {
        device_interface::ActiveModel {
            device_id: Set(device_id.clone()),
            interface_id: Set(interface.interface_id),
            r#type: Set(Some(interface.interface_type)),
            status: Set(Some(device_interface::DeviceInterfaceStatus::Unknown)),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&transaction)
        .await?;
    }

    if let (Some(device_type), Some(device_family)) = (
        registration.device_type.as_deref(),
        registration.device_family.as_deref(),
    ) {
        device_type_folder::Entity::insert(device_type_folder::ActiveModel {
            device_type: Set(device_type.to_owned()),
            folder_name: Set(format!("{device_family}_{device_type}")),
            device_family: Set(device_family.to_owned()),
            default_version: Set(registration
                .software_version
                .as_deref()
                .unwrap_or("v1.0.0")
                .to_owned()),
        })
        .on_conflict(
            OnConflict::column(device_type_folder::Column::DeviceType)
                .do_nothing()
                .to_owned(),
        )
        .exec(&transaction)
        .await?;
    }

    let device_model = device::Entity::find()
        .filter(device::Column::DeviceId.eq(&device_id))
        .one(&transaction)
        .await?
        .ok_or_else(|| DeviceRepositoryError::Internal("registered device disappeared".to_owned()))?;
    let interfaces = device_interface::Entity::find()
        .filter(device_interface::Column::DeviceId.eq(&device_id))
        .order_by_asc(device_interface::Column::Id)
        .all(&transaction)
        .await?;
    let mut device_value = device_json(&device_model);
    device_value["interfaces"] = serde_json::Value::Array(
        interfaces.iter().map(interface_json).collect(),
    );

    device::Entity::update_many()
        .col_expr(
            device::Column::Status,
            crate::persistence::pg_enum::pg_enum(&device::DeviceStatus::Approved),
        )
        .col_expr(
            device::Column::StatusActionBy,
            Expr::value(Option::<String>::None),
        )
        .col_expr(device::Column::UpdatedAt, Expr::value(now))
        .filter(device::Column::DeviceId.eq(&device_id))
        .filter(device::Column::Status.eq(device::DeviceStatus::Requested))
        .exec(&transaction)
        .await?;
    if notify_approval
        && registration.device_family.as_deref().is_some_and(|family| {
            family
                .split_whitespace()
                .collect::<String>()
                .eq_ignore_ascii_case("gen5")
        })
    {
        let active_exists = gen5_mapping_queue::Entity::find()
            .filter(Expr::cust_with_values(
                r#"regexp_replace(lower("macAddress"), '[:-]', '', 'g') = $1"#,
                [device_id.clone()],
            ))
            .filter(gen5_mapping_queue::Column::Status.is_in([
                gen5_mapping_queue::QueueJobStatus::Queued,
                gen5_mapping_queue::QueueJobStatus::Running,
                gen5_mapping_queue::QueueJobStatus::Waiting,
                gen5_mapping_queue::QueueJobStatus::FailurePolling,
            ]))
            .limit(1)
            .one(&transaction)
            .await?
            .is_some();
        if !active_exists {
            gen5_mapping_queue::ActiveModel {
                id: Set(Uuid::new_v4()),
                mac_address: Set(registration.mac_address.clone()),
                user_id: Set("system".to_owned()),
                status: Set(gen5_mapping_queue::QueueJobStatus::Queued),
                retry_count: Set(0),
                last_error: Set(None),
                failure_dc_ip: Set(None),
                baseline_mapping_count: Set(None),
                next_retry_at: Set(None),
                retry_schedule_index: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&transaction)
            .await?;
        }
    }
    transaction.commit().await?;
    if registration.device_family.as_deref().is_some_and(|family| {
        family
            .split_whitespace()
            .collect::<String>()
            .eq_ignore_ascii_case("gen3")
    }) {
        match add_gen3_controller_mapping(orm, &device_id).await {
            Ok(false) => {
                tracing::warn!(
                    controller_id = GEN3_CONTROLLER_ID,
                    mac_address = %registration.mac_address,
                    "Gen3 controller mapping was already present or the controller was unavailable"
                );
            }
            Ok(true) => {}
            Err(error) => {
                tracing::warn!(
                    %error,
                    controller_id = GEN3_CONTROLLER_ID,
                    mac_address = %registration.mac_address,
                    "failed to add best-effort Gen3 controller mapping"
                );
            }
        }
    }
    Ok(DeviceRegistrationResult {
        device: device_value,
        notify_addition,
        notify_approval,
    })
}

/// PURPOSE: Implement `add_gen3_controller_mapping` domain/persistence logic.
/// USE: Called as `add_gen3_controller_mapping` from handlers, repository impls, or sibling helpers.
async fn add_gen3_controller_mapping(
    orm: &DatabaseConnection,
    device_id: &str,
) -> Result<bool, DeviceRepositoryError> {
    let Some(controller) = device_controller::Entity::find_by_id(GEN3_CONTROLLER_ID.to_owned())
        .one(orm)
        .await?
    else {
        return Ok(false);
    };
    let mut mappings = if controller.mappings.is_null() {
        serde_json::json!({})
    } else {
        controller.mappings.clone()
    };
    let Some(object) = mappings.as_object_mut() else {
        return Ok(false);
    };
    if object.contains_key(device_id) {
        return Ok(false);
    }
    object.insert(
        device_id.to_owned(),
        serde_json::json!({"uart": "", "power": ""}),
    );
    let now = chrono::Utc::now();
    device_controller::ActiveModel {
        device_controller_id: Set(controller.device_controller_id),
        mappings: Set(mappings),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(orm)
    .await?;
    Ok(true)
}
