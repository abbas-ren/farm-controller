use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::repository_types::{
    ControllerHeartbeatResult, DeferredCancellation, DeviceHeartbeatEvent, DeviceHeartbeatResult,
};
use super::{
    ControllerHeartbeat, error::DeviceRepositoryError, validation::normalized_controller_id,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct InterfaceStatus {
    interface_type: String,
    interface_id: Option<String>,
    status: &'static str,
}

#[derive(Clone, Debug)]
struct InterfaceChange {
    interface_type: String,
    interface_id: String,
    status: &'static str,
    previous_status: Option<String>,
}

pub(crate) async fn record_controller(
    pool: &PgPool,
    heartbeat: &ControllerHeartbeat,
) -> Result<ControllerHeartbeatResult, DeviceRepositoryError> {
    let controller_id = normalized_controller_id(&heartbeat.uid)?;
    let timestamp = DateTime::<Utc>::from_timestamp(heartbeat.timestamp, 0).ok_or_else(|| {
        DeviceRepositoryError::Validation("heartbeat timestamp is out of range".to_owned())
    })?;
    let mut transaction = pool.begin().await?;
    let previous_state = sqlx::query_scalar::<_, String>(
        r#"SELECT state::text FROM device_controllers
           WHERE "deviceControllerId" = $1 AND "deletedAt" IS NULL FOR UPDATE"#,
    )
    .bind(&controller_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(previous_state) = previous_state else {
        transaction.commit().await?;
        return Ok(ControllerHeartbeatResult {
            state_changed: false,
            alert: None,
        });
    };
    sqlx::query(
        r#"
        UPDATE device_controllers
        SET state = 'active', "ipAddress" = $2, "updatedAt" = now()
        WHERE "deviceControllerId" = $1
        "#,
    )
    .bind(&controller_id)
    .bind(&heartbeat.ip)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO device_controller_heartbeats
            ("controllerId", timestamp, timeout, data)
        VALUES ($1, $2, 0, $3)
        "#,
    )
    .bind(&controller_id)
    .bind(timestamp)
    .bind(sqlx::types::Json(serde_json::to_value(heartbeat).map_err(
        |error| DeviceRepositoryError::Validation(error.to_string()),
    )?))
    .execute(&mut *transaction)
    .await?;

    let metrics_updated = sqlx::query(
        r#"
        UPDATE device_controller_metrics SET
            "cpuCurrent" = $2, "cpuTotal" = $3, "cpuUsagePercent" = $4,
            "memoryUsed" = $5, "memoryTotal" = $6, "memoryUsagePercent" = $7,
            "networkUpload" = $8, "networkDownload" = $9,
            "diskUsed" = $10, "diskTotal" = $11, "diskUsagePercent" = $12,
            "collectedAt" = $13, "updatedAt" = now()
        WHERE id = (
            SELECT id FROM device_controller_metrics
            WHERE "controllerId" = $1
            ORDER BY "updatedAt" DESC, id DESC LIMIT 1 FOR UPDATE
        )
        "#,
    )
    .bind(&controller_id)
    .bind(&heartbeat.cpu_current)
    .bind(&heartbeat.cpu_total)
    .bind(heartbeat.cpu_usage_percent)
    .bind(&heartbeat.memory_used)
    .bind(&heartbeat.memory_total)
    .bind(heartbeat.memory_usage_percent)
    .bind(&heartbeat.network_upload)
    .bind(&heartbeat.network_download)
    .bind(&heartbeat.disk_used)
    .bind(&heartbeat.disk_total)
    .bind(heartbeat.disk_usage_percent)
    .bind(timestamp)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if metrics_updated == 0 {
        sqlx::query(
            r#"
            INSERT INTO device_controller_metrics (
                "controllerId", "cpuCurrent", "cpuTotal", "cpuUsagePercent",
                "memoryUsed", "memoryTotal", "memoryUsagePercent",
                "networkUpload", "networkDownload", "diskUsed", "diskTotal",
                "diskUsagePercent", "collectedAt", "createdAt", "updatedAt"
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,now(),now())
            "#,
        )
        .bind(&controller_id)
        .bind(&heartbeat.cpu_current)
        .bind(&heartbeat.cpu_total)
        .bind(heartbeat.cpu_usage_percent)
        .bind(&heartbeat.memory_used)
        .bind(&heartbeat.memory_total)
        .bind(heartbeat.memory_usage_percent)
        .bind(&heartbeat.network_upload)
        .bind(&heartbeat.network_download)
        .bind(&heartbeat.disk_used)
        .bind(&heartbeat.disk_total)
        .bind(heartbeat.disk_usage_percent)
        .bind(timestamp)
        .execute(&mut *transaction)
        .await?;
    }
    let state_changed = previous_state != "active";
    let alert = if state_changed {
        Some(
            sqlx::query_scalar::<_, serde_json::Value>(
                r#"INSERT INTO alerts
                      (id, user_id, title, message, type, status, is_read,
                       device_controller_id, data, created_at, updated_at)
                   VALUES (gen_random_uuid(), 'ADMIN', 'Device Controller Online',
                           'Device controller ' || $1 || ' changed from not reachable to online',
                           'success', 'unread', false, $1,
                           jsonb_build_object('deviceControllerId', $1,
                                              'previousState', $2, 'state', 'active'),
                           now(), now())
                   RETURNING to_jsonb(alerts)"#,
            )
            .bind(&controller_id)
            .bind(&previous_state)
            .fetch_one(&mut *transaction)
            .await?,
        )
    } else {
        None
    };
    transaction.commit().await?;
    Ok(ControllerHeartbeatResult {
        state_changed,
        alert,
    })
}

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
    let heartbeat_data = interface_heartbeat_data(heartbeat);
    let interface_statuses = parse_interface_statuses(&heartbeat_data);
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
        .await?;
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
    for interface in interface_statuses {
        if let Some(interface_id) = interface.interface_id.as_deref() {
            let existing = sqlx::query_as::<_, (i32, String)>(
                r#"SELECT id, status::text FROM device_interfaces
                   WHERE "deviceId" = $1 AND type = $2 AND "interfaceId" = $3
                   FOR UPDATE"#,
            )
            .bind(device_id)
            .bind(&interface.interface_type)
            .bind(interface_id)
            .fetch_optional(&mut *transaction)
            .await?;
            match existing {
                Some((row_id, previous_status)) if previous_status != interface.status => {
                    update_interface_status(&mut transaction, row_id, interface.status).await?;
                    interface_changes.push(InterfaceChange {
                        interface_type: interface.interface_type,
                        interface_id: interface_id.to_owned(),
                        status: interface.status,
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
                    .bind(&interface.interface_type)
                    .bind(interface.status)
                    .execute(&mut *transaction)
                    .await?;
                    interface_changes.push(InterfaceChange {
                        interface_type: interface.interface_type,
                        interface_id: interface_id.to_owned(),
                        status: interface.status,
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
            .bind(&interface.interface_type)
            .fetch_all(&mut *transaction)
            .await?;
            for (row_id, interface_id, previous_status) in existing {
                if previous_status == interface.status {
                    continue;
                }
                update_interface_status(&mut transaction, row_id, interface.status).await?;
                interface_changes.push(InterfaceChange {
                    interface_type: interface.interface_type.clone(),
                    interface_id,
                    status: interface.status,
                    previous_status: Some(previous_status),
                });
            }
        }
    }

    for change in &interface_changes {
        if is_healthy_interface_status(change.status) {
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
    Ok(Some(DeviceHeartbeatResult {
        state: state.to_owned(),
        state_changed,
        interface_changes: interface_changes
            .into_iter()
            .map(interface_change_value)
            .collect(),
        heartbeat_data,
        alerts,
        post_update_events,
        deferred_cancellations,
    }))
}

async fn process_device_post_update(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    device_id: &str,
    nfs_path: Option<&str>,
) -> Result<(Vec<DeviceHeartbeatEvent>, Vec<DeferredCancellation>), sqlx::Error> {
    let mut events = Vec::new();
    let mut deferred_cancellations = Vec::new();
    let fallback = sqlx::query_as::<_, (i32, String, String)>(
        r#"SELECT id, "testId", "fallbackPath" FROM fallback_updates
           WHERE "deviceId" = $1 AND status::text IN ('pending', 'flashing')
           ORDER BY "createdAt" DESC LIMIT 1 FOR UPDATE"#,
    )
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some((fallback_id, test_id, fallback_path)) = fallback {
        let matched = nfs_path == Some(fallback_path.as_str());
        let phase = if matched {
            "COMPLETE_SUCCESS"
        } else {
            "COMPLETE_FAILED"
        };
        let fallback_update = if matched {
            r#"UPDATE fallback_updates SET status = 'completed', "updatedAt" = now()
               WHERE id = $1"#
        } else {
            r#"UPDATE fallback_updates SET status = 'failed', "updatedAt" = now()
               WHERE id = $1"#
        };
        sqlx::query(fallback_update)
            .bind(fallback_id)
            .execute(&mut **transaction)
            .await?;
        let execution_update = if matched {
            r#"UPDATE test_executions SET "executionPhase" = 'COMPLETE_SUCCESS',
                                          "updatedAt" = now()
               WHERE "testId" = $1"#
        } else {
            r#"UPDATE test_executions SET "executionPhase" = 'COMPLETE_FAILED',
                                          "updatedAt" = now()
               WHERE "testId" = $1"#
        };
        sqlx::query(execution_update)
            .bind(&test_id)
            .execute(&mut **transaction)
            .await?;
        events.push(DeviceHeartbeatEvent {
            event: "test_execution_update".to_owned(),
            payload: serde_json::json!({
                "testId": test_id,
                "update": {"type": "phase", "data": {"executionPhase": phase}},
            }),
            room: Some(format!("test:{test_id}")),
        });
    }

    let completed_flash = sqlx::query_scalar::<_, uuid::Uuid>(
        r#"UPDATE device_action_queue SET status = 'completed', "updatedAt" = now()
           WHERE id = (
               SELECT id FROM device_action_queue
               WHERE "targetDeviceId" = $1 AND type::text = 'flash'
                 AND status::text = 'running'
               ORDER BY "createdAt" ASC LIMIT 1 FOR UPDATE
           )
           RETURNING id"#,
    )
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if completed_flash.is_some()
        && let Some((release_id, device_type)) = sqlx::query_as::<_, (String, String)>(
            r#"UPDATE releases release
               SET status = 'passed', "isFaulty" = false, "updatedAt" = now()
               FROM devices device
               WHERE release."testedDeviceId" = $1 AND release.status::text = 'testing'
                 AND device."deviceId" = $1
               RETURNING release.id::text, device."deviceType""#,
        )
        .bind(device_id)
        .fetch_optional(&mut **transaction)
        .await?
    {
        events.push(DeviceHeartbeatEvent {
            event: "release-status-changed".to_owned(),
            payload: serde_json::json!({
                "releaseId": release_id,
                "status": "passed",
                "deviceId": device_id,
                "deviceType": device_type,
            }),
            room: None,
        });
    }

    let flash_confirm = sqlx::query_as::<_, (uuid::Uuid, Option<String>)>(
        r#"SELECT id, "testId" FROM device_action_queue
           WHERE "targetDeviceId" = $1 AND status::text = 'waiting'
             AND "iplFlashConfirm" = true
           ORDER BY "createdAt" ASC LIMIT 1 FOR UPDATE"#,
    )
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some((action_id, test_id)) = flash_confirm {
        let can_requeue = if let Some(test_id) = test_id.as_deref() {
            sqlx::query_scalar::<_, bool>(
                r#"SELECT COALESCE(NOT "cancelRequested", true)
                   FROM test_executions WHERE "testId" = $1 FOR UPDATE"#,
            )
            .bind(test_id)
            .fetch_optional(&mut **transaction)
            .await?
            .unwrap_or(false)
        } else {
            true
        };
        if can_requeue {
            sqlx::query(
                r#"UPDATE device_action_queue
                   SET status = 'queued', mode = 'deviceSpecific',
                       "targetDeviceId" = $2, "updatedAt" = now()
                   WHERE id = $1"#,
            )
            .bind(action_id)
            .bind(device_id)
            .execute(&mut **transaction)
            .await?;
            if let Some(test_id) = test_id.as_deref() {
                sqlx::query(
                    r#"UPDATE test_executions
                       SET "executionPhase" = 'REQUEUE_AFTER_IPL_CONFIRMATION',
                           "updatedAt" = now()
                       WHERE "testId" = $1"#,
                )
                .bind(test_id)
                .execute(&mut **transaction)
                .await?;
            }
                } else if let Some(test_id) = test_id.as_deref()
                        && let Some((build_id, created_by)) =
                                sqlx::query_as::<_, (String, Option<String>)>(
                                        r#"UPDATE test_executions
                       SET status = 'cancelled', "cancelHandled" = true,
                           "executionPhase" = 'COMPLETE_CANCELLED',
                           "startedAt" = COALESCE("startedAt", now()), "endedAt" = now(),
                           "updatedAt" = now()
                                             WHERE "testId" = $1
                                                 AND status::text NOT IN ('cancelled', 'completed', 'failed')
                                               RETURNING "buildId"::text, "createdBy""#,
                )
                .bind(test_id)
                .fetch_optional(&mut **transaction)
                .await?
        {
            sqlx::query(
                r#"UPDATE device_action_queue SET status = 'cancelled', "updatedAt" = now()
                   WHERE "testId" = $1
                     AND status::text NOT IN ('completed', 'cancelled')"#,
            )
            .bind(test_id)
            .execute(&mut **transaction)
            .await?;
            sqlx::query(
                r#"INSERT INTO log_entries
                      (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                   VALUES ('test', $1, jsonb_build_object('message', $2::text), 'info',
                           now(), now(), now())"#,
            )
            .bind(test_id)
            .bind("This test has been cancelled successfully")
            .execute(&mut **transaction)
            .await?;
            deferred_cancellations.push(DeferredCancellation {
                test_id: test_id.to_owned(),
                build_id,
                created_by: created_by.unwrap_or_else(|| "system".to_owned()),
            });
        }
        if let Some(test_id) = test_id.as_deref() {
            sqlx::query(
                r#"INSERT INTO log_entries
                      (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                   VALUES ('test', $1, jsonb_build_object('message', $2::text), 'info',
                           now(), now(), now())"#,
            )
            .bind(test_id)
            .bind(if can_requeue {
                format!("Device {device_id} is back online after IPL flashing, continuing the execution")
            } else {
                format!("Device {device_id} is back online after IPL flashing; requeue skipped due to cancellation request")
            })
            .execute(&mut **transaction)
            .await?;
        }
        return Ok((events, deferred_cancellations));
    }

    let running_test = sqlx::query_scalar::<_, String>(
        r#"SELECT "testId" FROM device_action_queue
           WHERE "targetDeviceId" = $1 AND type::text = 'test'
             AND status::text = 'running' AND "testId" IS NOT NULL
           ORDER BY "createdAt" ASC LIMIT 1"#,
    )
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(test_id) = running_test {
        sqlx::query(
            r#"UPDATE test_executions
               SET "executionPhase" = 'START_TEST_EXECUTION', "updatedAt" = now()
               WHERE "testId" = $1 AND COALESCE("cancelRequested", false) = false"#,
        )
        .bind(&test_id)
        .execute(&mut **transaction)
        .await?;
    }
    Ok((events, deferred_cancellations))
}

async fn update_interface_status(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row_id: i32,
    status: &str,
) -> Result<(), DeviceRepositoryError> {
    sqlx::query(
        r#"UPDATE device_interfaces
           SET status = ($2::text)::"enum_device_interfaces_status", "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(row_id)
    .bind(status)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn interface_heartbeat_data(heartbeat: &serde_json::Value) -> serde_json::Value {
    let mut data = heartbeat.get("data").cloned().unwrap_or_default();
    if let Some(encoded) = data.get("data").and_then(serde_json::Value::as_str)
        && let Ok(decoded) = serde_json::from_str(encoded)
    {
        data = decoded;
    }
    data
}

fn parse_interface_statuses(data: &serde_json::Value) -> Vec<InterfaceStatus> {
    let Some(categories) = data.as_object() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for (interface_type, interfaces) in categories {
        if interfaces.is_null()
            || interfaces
                .as_object()
                .is_some_and(|value| value.contains_key("not present"))
        {
            result.push(InterfaceStatus {
                interface_type: interface_type.clone(),
                interface_id: None,
                status: "not_available",
            });
            continue;
        }
        let Some(entries) = interfaces.as_object() else {
            continue;
        };
        for (interface_id, value) in entries {
            result.push(InterfaceStatus {
                interface_type: interface_type.clone(),
                interface_id: Some(interface_id.clone()),
                status: resolve_interface_status(value),
            });
        }
    }
    result
}

fn resolve_interface_status(data: &serde_json::Value) -> &'static str {
    let Some(entry) = data.as_object() else {
        return "unknown";
    };
    if let Some(slaves) = entry.get("slave").and_then(serde_json::Value::as_array) {
        return if slaves.is_empty() {
            "not_available"
        } else {
            "available"
        };
    }
    if entry
        .get("temp")
        .and_then(serde_json::Value::as_f64)
        .is_some()
    {
        return "available";
    }
    match entry
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("up") => "up",
        Some("down") => "down",
        Some("connected") => "connected",
        Some("disconnected") => "disconnected",
        _ => "unknown",
    }
}

fn is_healthy_interface_status(status: &str) -> bool {
    matches!(status, "up" | "connected" | "available" | "plugged")
}

fn interface_change_value(change: InterfaceChange) -> serde_json::Value {
    let mut value = serde_json::json!({
        "type": change.interface_type,
        "interfaceId": change.interface_id,
        "status": change.status,
    });
    if let Some(previous_status) = change.previous_status {
        value["previousStatus"] = serde_json::Value::String(previous_status);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_device_interface_heartbeat_shapes() {
        let heartbeat = serde_json::json!({
            "type": "heartbeat",
            "data": {
                "ethernet": {"eth0": {"status": "UP"}},
                "i2c": {"i2c-1": {"slave": ["0x20"]}},
                "temperature": {"cpu": {"temp": 41.5}},
                "usb": {"not present": " "},
                "serial": {"ttyS0": {"status": "disconnected"}}
            }
        });
        let data = interface_heartbeat_data(&heartbeat);
        assert_eq!(
            parse_interface_statuses(&data),
            vec![
                InterfaceStatus {
                    interface_type: "ethernet".to_owned(),
                    interface_id: Some("eth0".to_owned()),
                    status: "up"
                },
                InterfaceStatus {
                    interface_type: "i2c".to_owned(),
                    interface_id: Some("i2c-1".to_owned()),
                    status: "available"
                },
                InterfaceStatus {
                    interface_type: "serial".to_owned(),
                    interface_id: Some("ttyS0".to_owned()),
                    status: "disconnected"
                },
                InterfaceStatus {
                    interface_type: "temperature".to_owned(),
                    interface_id: Some("cpu".to_owned()),
                    status: "available"
                },
                InterfaceStatus {
                    interface_type: "usb".to_owned(),
                    interface_id: None,
                    status: "not_available"
                },
            ]
        );
    }

    #[test]
    fn decodes_nested_stringified_interface_data() {
        let heartbeat =
            serde_json::json!({"data": {"data": "{\"uart\":{\"ttyS1\":{\"status\":\"down\"}}}"}});
        assert_eq!(
            parse_interface_statuses(&interface_heartbeat_data(&heartbeat)),
            vec![InterfaceStatus {
                interface_type: "uart".to_owned(),
                interface_id: Some("ttyS1".to_owned()),
                status: "down",
            }]
        );
    }
}
