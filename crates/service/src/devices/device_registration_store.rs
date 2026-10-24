use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection,
    EntityTrait, FromQueryResult, QueryFilter, Set, Statement, TransactionTrait, sea_query::Expr,
};

use crate::persistence::entity::{alert, device, device_state_change};

use super::{
    constants::GEN3_CONTROLLER_ID,
    error::DeviceRepositoryError,
    repository_types::DeviceRegistrationResult,
    types::DeviceRegistration,
    validation::{device_interfaces, validate_device_registration},
};

#[derive(Debug, FromQueryResult)]
struct ExistingDevice {
    active: bool,
    status: String,
}

#[derive(Debug, FromQueryResult)]
struct JsonRow {
    value: serde_json::Value,
}

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
    let existing = ExistingDevice::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT "deletedAt" IS NULL AS active, COALESCE(status::text, 'requested') AS status
           FROM devices WHERE "deviceId" = $1 FOR UPDATE"#,
        [device_id.clone().into()],
    ))
    .one(&transaction)
    .await?;
    let notify_addition = existing.as_ref().is_none_or(|row| !row.active);
    let notify_approval = notify_addition
        || existing
            .as_ref()
            .is_some_and(|row| row.active && row.status == "requested");

    let now = chrono::Utc::now();
    match existing.as_ref() {
        Some(row) if row.active => {
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
                .col_expr(device::Column::Status, Expr::value(Some(status)))
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
                    Expr::value(Some(device::DeviceState::Unknown)),
                )
                .col_expr(
                    device::Column::Status,
                    Expr::value(Some(device::DeviceStatus::Requested)),
                )
                .col_expr(device::Column::ControllerId, Expr::value(Some(String::new())))
                .col_expr(device::Column::Upgrading, Expr::value(Some(false)))
                .col_expr(device::Column::Flashing, Expr::value(Some(false)))
                .col_expr(device::Column::DeletedAt, Expr::value(Option::<chrono::DateTime<chrono::Utc>>::None))
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
            // Insert via raw SQL: devices.id is serial and DeviceId is unique business key.
            transaction
                .execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    r#"INSERT INTO devices
                       ("deviceId", "macAddress", "ipAddress", "deviceName", "deviceFamily",
                        "deviceType", "buildId", "heartbeatTimer", "softwareVersion", "nfsPath",
                        state, status, "controllerId", upgrading, flashing, "createdAt", "updatedAt")
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                           $11::"enum_devices_state", $12::"enum_devices_status", '', false, false,
                           now(), now())"#,
                    [
                        device_id.clone().into(),
                        registration.mac_address.clone().into(),
                        registration.ip_address.clone().into(),
                        registration.device_name.clone().into(),
                        registration.device_family.clone().into(),
                        registration.device_type.clone().into(),
                        registration.build_id.clone().into(),
                        registration.timeout.into(),
                        registration.software_version.clone().into(),
                        registration.nfs_path.clone().into(),
                        match state {
                            device::DeviceState::Busy => "busy",
                            device::DeviceState::NotReachable => "not_reachable",
                            device::DeviceState::Free => "free",
                            device::DeviceState::Faulty => "faulty",
                            device::DeviceState::Unknown => "unknown",
                        }
                        .into(),
                        match status {
                            device::DeviceStatus::Approved => "approved",
                            device::DeviceStatus::Requested => "requested",
                            device::DeviceStatus::Declined => "declined",
                        }
                        .into(),
                    ],
                ))
                .await?;
            alert::ActiveModel {
                id: Set(uuid::Uuid::new_v4()),
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

    // Legacy `type` column vs entity `interfaceType` — keep as raw SQL.
    transaction
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"DELETE FROM device_interfaces WHERE "deviceId" = $1"#,
            [device_id.clone().into()],
        ))
        .await?;
    for interface in device_interfaces(registration) {
        transaction
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"INSERT INTO device_interfaces
                   ("deviceId", type, "interfaceId", "createdAt", "updatedAt")
               VALUES ($1, $2, $3, now(), now())"#,
                [
                    device_id.clone().into(),
                    interface.interface_type.into(),
                    interface.interface_id.into(),
                ],
            ))
            .await?;
    }

    if let (Some(device_type), Some(device_family)) = (
        registration.device_type.as_deref(),
        registration.device_family.as_deref(),
    ) {
        transaction
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"INSERT INTO device_type_folders
                   ("deviceType", "folderName", "deviceFamily", "defaultVersion")
               VALUES ($1, $2, $3, $4)
               ON CONFLICT ("deviceType") DO NOTHING"#,
                [
                    device_type.into(),
                    format!("{device_family}_{device_type}").into(),
                    device_family.into(),
                    registration
                        .software_version
                        .as_deref()
                        .unwrap_or("v1.0.0")
                        .into(),
                ],
            ))
            .await?;
    }

    let device_json = JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(device_row) || jsonb_build_object(
                   'interfaces', COALESCE((
                       SELECT jsonb_agg(to_jsonb(interface_row) ORDER BY interface_row.id)
                       FROM device_interfaces interface_row
                       WHERE interface_row."deviceId" = device_row."deviceId"
                   ), '[]'::jsonb)
               ) AS value
           FROM devices device_row WHERE device_row."deviceId" = $1"#,
        [device_id.clone().into()],
    ))
    .one(&transaction)
    .await?
    .ok_or_else(|| DeviceRepositoryError::Internal("registered device disappeared".to_owned()))?
    .value;

    device::Entity::update_many()
        .col_expr(
            device::Column::Status,
            Expr::value(Some(device::DeviceStatus::Approved)),
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
        // Conditional insert with NOT EXISTS kept as raw SQL.
        transaction
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"INSERT INTO gen5_mapping_queue
                   (id, "macAddress", "userId", status, "retryCount", "createdAt", "updatedAt")
               SELECT gen_random_uuid(), $1, 'system', 'queued', 0, now(), now()
               WHERE NOT EXISTS (
                   SELECT 1 FROM gen5_mapping_queue
                   WHERE regexp_replace(lower("macAddress"), '[:-]', '', 'g') = $2
                     AND status::text IN ('queued', 'running', 'waiting', 'failure_polling')
               )"#,
                [
                    registration.mac_address.clone().into(),
                    device_id.clone().into(),
                ],
            ))
            .await?;
    }
    transaction.commit().await?;
    if registration.device_family.as_deref().is_some_and(|family| {
        family
            .split_whitespace()
            .collect::<String>()
            .eq_ignore_ascii_case("gen3")
    }) {
        match orm
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"UPDATE device_controllers
               SET mappings = COALESCE(mappings, '{}'::jsonb)
                              || jsonb_build_object($2, jsonb_build_object('uart', '', 'power', '')),
                   "updatedAt" = now()
               WHERE "deviceControllerId" = $1
                 AND NOT (COALESCE(mappings, '{}'::jsonb) ? $2)"#,
                [GEN3_CONTROLLER_ID.into(), device_id.clone().into()],
            ))
            .await
        {
            Ok(result) if result.rows_affected() == 0 => {
                tracing::warn!(
                    controller_id = GEN3_CONTROLLER_ID,
                    mac_address = %registration.mac_address,
                    "Gen3 controller mapping was already present or the controller was unavailable"
                );
            }
            Ok(_) => {}
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
        device: device_json,
        notify_addition,
        notify_approval,
    })
}
