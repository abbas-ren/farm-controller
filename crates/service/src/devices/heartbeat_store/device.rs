use sqlx::PgPool;

use crate::devices::repository_types::DeviceHeartbeatResult;

use super::super::error::DeviceRepositoryError;
use super::{interface, state_transition::process_device_post_update};

pub(crate) async fn record_device(
    pool: &PgPool,
    device_id: &str,
    heartbeat: &serde_json::Value,
) -> Result<Option<DeviceHeartbeatResult>, DeviceRepositoryError> {
    if !heartbeat.is_object() {
        return Err(DeviceRepositoryError::Validation(
            "device heartbeat must be a JSON object".to_owned(),
        ));
    }
    let heartbeat_data = interface::interface_heartbeat_data(heartbeat);
    let interface_statuses = interface::parse_interface_statuses(&heartbeat_data);
    let mut transaction = pool.begin().await?;
    let device = sqlx::query_as::<_, (String, bool)>(
        r#"SELECT device.state::text,
                  (EXISTS (SELECT 1 FROM device_action_queue action
                           WHERE action."targetDeviceId" = device."deviceId"
                             AND action.status::text IN ('queued', 'waiting', 'running'))
                   OR EXISTS (SELECT 1 FROM fallback_updates fallback
                              WHERE fallback."deviceId" = device."deviceId"
                                AND fallback.status::text IN ('pending', 'flashing')))
           FROM devices device
           WHERE device."deviceId" = $1
             AND device."deletedAt" IS NULL
             AND device.status::text = 'approved'
           FOR UPDATE OF device"#,
    )
    .bind(device_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((previous_state, has_active_work)) = device else {
        transaction.rollback().await?;
        return Ok(None);
    };

    let existing = sqlx::query_as::<_, (i64, serde_json::Value)>(
        r#"SELECT id::bigint, data FROM heartbeats
           WHERE "deviceId" = $1 ORDER BY timestamp DESC LIMIT 1 FOR UPDATE"#,
    )
    .bind(device_id)
    .fetch_optional(&mut *transaction)
    .await?;
    match existing {
        Some((heartbeat_id, data)) if data == *heartbeat => {
            sqlx::query("UPDATE heartbeats SET timestamp = now(), timeout = 0 WHERE id = $1")
                .bind(heartbeat_id)
                .execute(&mut *transaction)
                .await?;
        }
        _ => {
            sqlx::query(
                r#"INSERT INTO heartbeats ("deviceId", timestamp, timeout, data)
                   VALUES ($1, now(), 0, $2)"#,
            )
            .bind(device_id)
            .bind(sqlx::types::Json(heartbeat.clone()))
            .execute(&mut *transaction)
            .await?;
        }
    }

    let state = if has_active_work { "busy" } else { "free" };
    let state_update = if has_active_work {
        r#"UPDATE devices SET state = 'busy', "stateUpdatedAt" = now(),
                            "lastConnectedOn" = now(), upgrading = false,
                            flashing = false, "updatedAt" = now()
           WHERE "deviceId" = $1"#
    } else {
        r#"UPDATE devices SET state = 'free', "stateUpdatedAt" = now(),
                            "lastConnectedOn" = now(), upgrading = false,
                            flashing = false, "updatedAt" = now()
           WHERE "deviceId" = $1"#
    };
    sqlx::query(state_update)
        .bind(device_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            tracing::error!(%error, device_id, target_state = state, "failed to persist device heartbeat state transition");
            error
        })?;
    let state_change_insert = if has_active_work {
        r#"INSERT INTO device_state_change
              ("deviceId", state, "changedAt", "createdAt", "updatedAt")
           VALUES ($1, 'busy', now(), now(), now())"#
    } else {
        r#"INSERT INTO device_state_change
              ("deviceId", state, "changedAt", "createdAt", "updatedAt")
           VALUES ($1, 'free', now(), now(), now())"#
    };
    sqlx::query(state_change_insert)
        .bind(device_id)
        .execute(&mut *transaction)
        .await?;

    let mut alerts = Vec::new();
    if !has_active_work && matches!(previous_state.as_str(), "faulty" | "not_reachable") {
        alerts.push(
            sqlx::query_scalar::<_, serde_json::Value>(
                r#"INSERT INTO alerts
                      (id, user_id, title, message, type, status, is_read,
                       device_id, data, created_at, updated_at)
                   VALUES (gen_random_uuid(), 'ADMIN', 'Device comes as Available',
                           'Device ' || $1 || ' is now available', 'success', 'unread', false,
                           $1, jsonb_build_object('deviceId', $1, 'previousState', $2,
                                                  'state', 'free'), now(), now())
                   RETURNING to_jsonb(alerts)"#,
            )
            .bind(device_id)
            .bind(&previous_state)
            .fetch_one(&mut *transaction)
            .await?,
        );
    }

    let mut interface_changes = Vec::new();
    for interface_status in interface_statuses {
        if let Some(interface_id) = interface_status.interface_id.as_deref() {
            let existing = sqlx::query_as::<_, (i32, String)>(
                r#"SELECT id, status::text FROM device_interfaces
                   WHERE "deviceId" = $1 AND type = $2 AND "interfaceId" = $3
                   FOR UPDATE"#,
            )
            .bind(device_id)
            .bind(&interface_status.interface_type)
            .bind(interface_id)
            .fetch_optional(&mut *transaction)
            .await?;
            match existing {
                Some((row_id, previous_status)) if previous_status != interface_status.status => {
                    interface::update_interface_status(
                        &mut transaction,
                        row_id,
                        interface_status.status,
                    )
                    .await?;
                    interface_changes.push(interface::InterfaceChange {
                        interface_type: interface_status.interface_type,
                        interface_id: interface_id.to_owned(),
                        status: interface_status.status,
                        previous_status: Some(previous_status),
                    });
                }
                None => {
                    sqlx::query(
                        r#"INSERT INTO device_interfaces
                              ("deviceId", "interfaceId", type, status, "createdAt", "updatedAt")
                           VALUES ($1, $2, $3, ($4::text)::"enum_device_interfaces_status",
                                   now(), now())"#,
                    )
                    .bind(device_id)
                    .bind(interface_id)
                    .bind(&interface_status.interface_type)
                    .bind(interface_status.status)
                    .execute(&mut *transaction)
                    .await?;
                    interface_changes.push(interface::InterfaceChange {
                        interface_type: interface_status.interface_type,
                        interface_id: interface_id.to_owned(),
                        status: interface_status.status,
                        previous_status: None,
                    });
                }
                Some(_) => {}
            }
        } else {
            let existing = sqlx::query_as::<_, (i32, String, String)>(
                r#"SELECT id, "interfaceId", status::text FROM device_interfaces
                   WHERE "deviceId" = $1 AND type = $2 FOR UPDATE"#,
            )
            .bind(device_id)
            .bind(&interface_status.interface_type)
            .fetch_all(&mut *transaction)
            .await?;
            for (row_id, interface_id, previous_status) in existing {
                if previous_status == interface_status.status {
                    continue;
                }
                interface::update_interface_status(
                    &mut transaction,
                    row_id,
                    interface_status.status,
                )
                .await?;
                interface_changes.push(interface::InterfaceChange {
                    interface_type: interface_status.interface_type.clone(),
                    interface_id,
                    status: interface_status.status,
                    previous_status: Some(previous_status),
                });
            }
        }
    }

    for change in &interface_changes {
        if interface::is_healthy_interface_status(change.status) {
            continue;
        }
        let interface_label = format!("{}:{}", change.interface_type, change.interface_id);
        alerts.push(
            sqlx::query_scalar::<_, serde_json::Value>(
                r#"INSERT INTO alerts
                      (id, user_id, title, message, type, status, is_read,
                       device_id, data, created_at, updated_at)
                   VALUES (gen_random_uuid(), 'ADMIN', 'Interface ' || $2 || ' ' || $3,
                           'Device ' || $1 || ' interface ' || $2 || ' status is ' || $3,
                           'warning', 'unread', false, $1,
                           jsonb_build_object('alertCategory', 'interface-status',
                                              'deviceId', $1, 'interfaceType', $4,
                                              'interfaceId', $5, 'interfaceStatus', $3,
                                              'previousInterfaceStatus', $6::text),
                           now(), now())
                   RETURNING to_jsonb(alerts)"#,
            )
            .bind(device_id)
            .bind(&interface_label)
            .bind(change.status)
            .bind(&change.interface_type)
            .bind(&change.interface_id)
            .bind(change.previous_status.as_deref())
            .fetch_one(&mut *transaction)
            .await?,
        );
    }

    let (post_update_events, deferred_cancellations) = process_device_post_update(
        &mut transaction,
        device_id,
        heartbeat.get("nfsPath").and_then(serde_json::Value::as_str),
    )
    .await?;
    transaction.commit().await?;
    let state_changed = previous_state != state;
    if matches!(previous_state.as_str(), "faulty" | "not_reachable") && state == "free" {
        tracing::info!(
            device_id,
            previous_state,
            "device heartbeat recovered to free"
        );
    }
    Ok(Some(DeviceHeartbeatResult {
        state: state.to_owned(),
        state_changed,
        interface_changes: interface_changes
            .into_iter()
            .map(interface::interface_change_value)
            .collect(),
        heartbeat_data,
        alerts,
        post_update_events,
        deferred_cancellations,
    }))
}
