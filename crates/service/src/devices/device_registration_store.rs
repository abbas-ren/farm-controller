use sqlx::PgPool;

use super::{
    constants::GEN3_CONTROLLER_ID,
    error::DeviceRepositoryError,
    repository_types::DeviceRegistrationResult,
    types::DeviceRegistration,
    validation::{device_interfaces, validate_device_registration},
};

pub(crate) async fn register_device(
    pool: &PgPool,
    registration: &DeviceRegistration,
) -> Result<DeviceRegistrationResult, DeviceRepositoryError> {
    let device_id = validate_device_registration(registration)?;
    let mut transaction = pool.begin().await?;
    let existing = sqlx::query_as::<_, (bool, String)>(
        r#"SELECT "deletedAt" IS NULL, COALESCE(status::text, 'requested')
           FROM devices WHERE "deviceId" = $1 FOR UPDATE"#,
    )
    .bind(&device_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let notify_addition = existing.as_ref().is_none_or(|(active, _)| !active);
    let notify_approval = notify_addition
        || existing
            .as_ref()
            .is_some_and(|(active, status)| *active && status == "requested");

    match existing.as_ref() {
        Some((true, _)) => {
            sqlx::query(
                r#"UPDATE devices SET
                       "macAddress" = $2, "ipAddress" = $3, "deviceName" = $4,
                       "deviceFamily" = $5, "deviceType" = $6, "buildId" = $7,
                       "heartbeatTimer" = $8, "softwareVersion" = $9, "nfsPath" = $10,
                       status = $11::"enum_devices_status", "controllerId" = '',
                       "updatedAt" = now()
                   WHERE "deviceId" = $1"#,
            )
            .bind(&device_id)
            .bind(&registration.mac_address)
            .bind(&registration.ip_address)
            .bind(&registration.device_name)
            .bind(&registration.device_family)
            .bind(&registration.device_type)
            .bind(&registration.build_id)
            .bind(registration.timeout)
            .bind(&registration.software_version)
            .bind(&registration.nfs_path)
            .bind(&registration.status)
            .execute(&mut *transaction)
            .await?;
        }
        Some((false, _)) => {
            sqlx::query(
                r#"UPDATE devices SET
                       "macAddress" = $2, "ipAddress" = $3, "deviceName" = $4,
                       "deviceFamily" = $5, "deviceType" = $6, "buildId" = $7,
                       "heartbeatTimer" = $8, "softwareVersion" = $9, "nfsPath" = $10,
                       state = 'unknown', status = 'requested', "controllerId" = '',
                       upgrading = false, flashing = false, "deletedAt" = NULL,
                       "stateUpdatedAt" = now(), "updatedAt" = now()
                   WHERE "deviceId" = $1"#,
            )
            .bind(&device_id)
            .bind(&registration.mac_address)
            .bind(&registration.ip_address)
            .bind(&registration.device_name)
            .bind(&registration.device_family)
            .bind(&registration.device_type)
            .bind(&registration.build_id)
            .bind(registration.timeout)
            .bind(&registration.software_version)
            .bind(&registration.nfs_path)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                r#"INSERT INTO device_state_change
                       ("deviceId", state, "changedAt", "createdAt", "updatedAt")
                   VALUES ($1, 'unknown', now(), now(), now())"#,
            )
            .bind(&device_id)
            .execute(&mut *transaction)
            .await?;
        }
        None => {
            sqlx::query(
                r#"INSERT INTO devices
                       ("deviceId", "macAddress", "ipAddress", "deviceName", "deviceFamily",
                        "deviceType", "buildId", "heartbeatTimer", "softwareVersion", "nfsPath",
                        state, status, "controllerId", upgrading, flashing, "createdAt", "updatedAt")
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                           $11::"enum_devices_state", $12::"enum_devices_status", '', false, false,
                           now(), now())"#,
            )
            .bind(&device_id)
            .bind(&registration.mac_address)
            .bind(&registration.ip_address)
            .bind(&registration.device_name)
            .bind(&registration.device_family)
            .bind(&registration.device_type)
            .bind(&registration.build_id)
            .bind(registration.timeout)
            .bind(&registration.software_version)
            .bind(&registration.nfs_path)
            .bind(&registration.state)
            .bind(&registration.status)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                r#"INSERT INTO alerts
                      (id, user_id, title, message, type, status, is_read, device_id,
                       data, created_at, updated_at)
                   VALUES (gen_random_uuid(), 'ADMIN', 'New Device On board',
                           'New device onboarded: ' || $1, 'info', 'unread', false, $1,
                           jsonb_build_object('deviceId', $1), now(), now())"#,
            )
            .bind(&device_id)
            .execute(&mut *transaction)
            .await?;
        }
    }

    sqlx::query(r#"DELETE FROM device_interfaces WHERE "deviceId" = $1"#)
        .bind(&device_id)
        .execute(&mut *transaction)
        .await?;
    for interface in device_interfaces(registration) {
        sqlx::query(
            r#"INSERT INTO device_interfaces
                   ("deviceId", type, "interfaceId", "createdAt", "updatedAt")
               VALUES ($1, $2, $3, now(), now())"#,
        )
        .bind(&device_id)
        .bind(interface.interface_type)
        .bind(interface.interface_id)
        .execute(&mut *transaction)
        .await?;
    }

    if let (Some(device_type), Some(device_family)) = (
        registration.device_type.as_deref(),
        registration.device_family.as_deref(),
    ) {
        sqlx::query(
            r#"INSERT INTO device_type_folders
                   ("deviceType", "folderName", "deviceFamily", "defaultVersion")
               VALUES ($1, $2, $3, $4)
               ON CONFLICT ("deviceType") DO NOTHING"#,
        )
        .bind(device_type)
        .bind(format!("{device_family}_{device_type}"))
        .bind(device_family)
        .bind(registration.software_version.as_deref().unwrap_or("v1.0.0"))
        .execute(&mut *transaction)
        .await?;
    }

    let device = sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT to_jsonb(device_row) || jsonb_build_object(
                   'interfaces', COALESCE((
                       SELECT jsonb_agg(to_jsonb(interface_row) ORDER BY interface_row.id)
                       FROM device_interfaces interface_row
                       WHERE interface_row."deviceId" = device_row."deviceId"
                   ), '[]'::jsonb)
               )
           FROM devices device_row WHERE device_row."deviceId" = $1"#,
    )
    .bind(&device_id)
    .fetch_one(&mut *transaction)
    .await?;

    sqlx::query(
        r#"UPDATE devices SET status = 'approved', "statusActionBy" = NULL, "updatedAt" = now()
           WHERE "deviceId" = $1 AND status::text = 'requested'"#,
    )
    .bind(&device_id)
    .execute(&mut *transaction)
    .await?;
    if notify_approval
        && registration.device_family.as_deref().is_some_and(|family| {
            family
                .split_whitespace()
                .collect::<String>()
                .eq_ignore_ascii_case("gen5")
        })
    {
        sqlx::query(
            r#"INSERT INTO gen5_mapping_queue
                   (id, "macAddress", "userId", status, "retryCount", "createdAt", "updatedAt")
               SELECT gen_random_uuid(), $1, 'system', 'queued', 0, now(), now()
               WHERE NOT EXISTS (
                   SELECT 1 FROM gen5_mapping_queue
                   WHERE regexp_replace(lower("macAddress"), '[:-]', '', 'g') = $2
                     AND status::text IN ('queued', 'running', 'waiting', 'failure_polling')
               )"#,
        )
        .bind(&registration.mac_address)
        .bind(&device_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    if registration.device_family.as_deref().is_some_and(|family| {
        family
            .split_whitespace()
            .collect::<String>()
            .eq_ignore_ascii_case("gen3")
    }) {
        match sqlx::query(
            r#"UPDATE device_controllers
               SET mappings = COALESCE(mappings, '{}'::jsonb)
                              || jsonb_build_object($2, jsonb_build_object('uart', '', 'power', '')),
                   "updatedAt" = now()
               WHERE "deviceControllerId" = $1
                 AND NOT (COALESCE(mappings, '{}'::jsonb) ? $2)"#,
        )
        .bind(GEN3_CONTROLLER_ID)
        .bind(&device_id)
        .execute(pool)
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
        device,
        notify_addition,
        notify_approval,
    })
}
