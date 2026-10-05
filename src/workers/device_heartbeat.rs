use std::{sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;

use crate::{events::ServerEvent, state::AppState};

#[derive(Debug, sqlx::FromRow)]
struct StaleDevice {
    device_id: String,
    previous_state: String,
    target_state: String,
    timeout: i32,
}

struct DeviceStateTransition {
    device_id: String,
    state: String,
    alert: serde_json::Value,
}

pub(super) async fn worker(state: Arc<AppState>, cancellation: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.device_heartbeat_poll_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_cycle(&state).await,
        }
    }
    tracing::info!(worker = "device_heartbeat_timeout", "worker stopped");
}

async fn run_cycle(state: &AppState) {
    let Some(database) = &state.database else {
        return;
    };
    let mut transaction = match database.pool().begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            record_failure(state, &error, "failed to start timeout transaction");
            return;
        }
    };
    let batch_size = i64::from(state.config.workers.device_heartbeat_batch_size);
    let stale_devices = match sqlx::query_as::<_, StaleDevice>(
        r#"SELECT device."deviceId" AS device_id,
                  device.state::text AS previous_state,
                  target.target_state,
                  floor(extract(epoch FROM now() - seen.last_seen))::integer AS timeout
           FROM devices device
           CROSS JOIN LATERAL (
               SELECT COALESCE(
                   (SELECT max(heartbeat.timestamp)
                    FROM heartbeats heartbeat
                    WHERE heartbeat."deviceId" = device."deviceId"
                      AND NOT (heartbeat.data @> '{"status":"disconnected"}'::jsonb)),
                   device."lastConnectedOn", device."createdAt"
               ) AS last_seen
           ) seen
           CROSS JOIN LATERAL (
               SELECT CASE
                   WHEN seen.last_seen < now() -
                        (GREATEST(COALESCE(device."heartbeatTimer", 5), 1) * 12 * interval '1 second')
                   THEN 'faulty'
                   ELSE 'not_reachable'
               END AS target_state
           ) target
           WHERE device."deletedAt" IS NULL
             AND device.status::text = 'approved'
             AND seen.last_seen < now() -
                 (GREATEST(COALESCE(device."heartbeatTimer", 5), 1) * 6 * interval '1 second')
             AND device.state::text <> target.target_state
             AND NOT EXISTS (
                 SELECT 1 FROM device_action_queue action
                 WHERE action."targetDeviceId" = device."deviceId"
                   AND action.status::text IN ('queued', 'waiting', 'running')
             )
             AND NOT EXISTS (
                 SELECT 1 FROM fallback_updates fallback
                 WHERE fallback."deviceId" = device."deviceId"
                   AND fallback.status::text IN ('pending', 'flashing')
             )
           ORDER BY seen.last_seen ASC
           LIMIT $1
           FOR UPDATE OF device SKIP LOCKED"#,
    )
    .bind(batch_size)
    .fetch_all(&mut *transaction)
    .await
    {
        Ok(devices) => devices,
        Err(error) => {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to load stale devices");
            return;
        }
    };

    let mut transitions = Vec::with_capacity(stale_devices.len());
    for device in stale_devices {
        let update_query = if device.target_state == "faulty" {
            r#"UPDATE devices SET state = 'faulty', "stateUpdatedAt" = now(),
                                "updatedAt" = now()
               WHERE "deviceId" = $1"#
        } else {
            r#"UPDATE devices SET state = 'not_reachable', "stateUpdatedAt" = now(),
                                "updatedAt" = now()
               WHERE "deviceId" = $1"#
        };
        if let Err(error) = sqlx::query(update_query)
            .bind(&device.device_id)
            .execute(&mut *transaction)
            .await
        {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to update stale device");
            return;
        }
        let state_change_query = if device.target_state == "faulty" {
            r#"INSERT INTO device_state_change
                  ("deviceId", state, "changedAt", "createdAt", "updatedAt")
               VALUES ($1, 'faulty', now(), now(), now())"#
        } else {
            r#"INSERT INTO device_state_change
                  ("deviceId", state, "changedAt", "createdAt", "updatedAt")
               VALUES ($1, 'not_reachable', now(), now(), now())"#
        };
        if let Err(error) = sqlx::query(state_change_query)
            .bind(&device.device_id)
            .execute(&mut *transaction)
            .await
        {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to store device state history");
            return;
        }
        if let Err(error) = sqlx::query(
            r#"INSERT INTO heartbeats ("deviceId", timestamp, timeout, data)
               VALUES ($1, now(), $2, '{"status":"disconnected"}'::jsonb)"#,
        )
        .bind(&device.device_id)
        .bind(device.timeout)
        .execute(&mut *transaction)
        .await
        {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to store disconnected heartbeat");
            return;
        }
        let alert_query = if device.target_state == "faulty" {
            r#"INSERT INTO alerts
                  (id, user_id, title, message, type, status, is_read,
                   device_id, data, created_at, updated_at)
               VALUES (gen_random_uuid(), 'ADMIN', 'Device goes faulty',
                       'Device ' || $1 || ' is faulty', 'error', 'unread', false, $1,
                       jsonb_build_object('deviceId', $1, 'previousState', $2,
                                          'state', 'faulty'), now(), now())
               RETURNING to_jsonb(alerts)"#
        } else {
            r#"INSERT INTO alerts
                  (id, user_id, title, message, type, status, is_read,
                   device_id, data, created_at, updated_at)
               VALUES (gen_random_uuid(), 'ADMIN',
                       'Device ' || $1 || ' is not_reachable',
                       'Device ' || $1 || ' is not_reachable',
                       'warning', 'unread', false, $1,
                       jsonb_build_object('deviceId', $1, 'previousState', $2,
                                          'state', 'not_reachable'), now(), now())
               RETURNING to_jsonb(alerts)"#
        };
        let alert = match sqlx::query_scalar::<_, serde_json::Value>(alert_query)
            .bind(&device.device_id)
            .bind(&device.previous_state)
            .fetch_one(&mut *transaction)
            .await
        {
            Ok(alert) => alert,
            Err(error) => {
                let _ = transaction.rollback().await;
                record_failure(state, &error, "failed to store device timeout alert");
                return;
            }
        };
        transitions.push(DeviceStateTransition {
            device_id: device.device_id,
            state: device.target_state,
            alert,
        });
    }

    if let Err(error) = transaction.commit().await {
        record_failure(state, &error, "failed to commit device heartbeat timeouts");
        return;
    }
    for transition in &transitions {
        for event in transition_events(transition) {
            if let Err(error) = state.event_publisher.publish(event) {
                tracing::warn!(%error, device_id = transition.device_id, "failed to publish device timeout");
            }
        }
    }
    state
        .metrics
        .record_worker_items("device_heartbeat_timeout", transitions.len() as u64);
    state
        .metrics
        .record_worker_run("device_heartbeat_timeout", "success");
}

fn transition_events(transition: &DeviceStateTransition) -> [ServerEvent; 2] {
    [
        ServerEvent {
            event: "device_state_update".to_owned(),
            payload: serde_json::json!({
                "deviceId": transition.device_id,
                "state": transition.state,
                "upgrading": false,
                "flashing": false,
                "changedAt": chrono::Utc::now().to_rfc3339(),
            }),
            room: None,
        },
        ServerEvent::alert("device-alert", transition.alert.clone()),
    ]
}

fn record_failure(state: &AppState, error: &sqlx::Error, message: &str) {
    state
        .metrics
        .record_worker_run("device_heartbeat_timeout", "failure");
    tracing::error!(%error, worker = "device_heartbeat_timeout", "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_events_preserve_device_frontend_contracts() {
        let [state, alert] = transition_events(&DeviceStateTransition {
            device_id: "device-1".to_owned(),
            state: "not_reachable".to_owned(),
            alert: serde_json::json!({"title": "Device device-1 is not_reachable"}),
        });
        assert_eq!(state.event, "device_state_update");
        assert_eq!(state.payload["deviceId"], "device-1");
        assert_eq!(state.payload["state"], "not_reachable");
        assert!(state.room.is_none());
        assert_eq!(alert.event, "alert");
        assert_eq!(alert.payload["subtype"], "device-alert");
    }
}
