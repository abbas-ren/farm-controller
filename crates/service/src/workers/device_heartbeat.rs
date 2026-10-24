use std::{sync::Arc, time::Duration};

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseBackend, EntityTrait, FromQueryResult, QueryFilter, Set,
    Statement, TransactionTrait,
    sea_query::Expr,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    persistence::entity::{alert, device, device_state_change, heartbeat},
    socket::helpers::transition_events,
    state::AppState,
    workers::types::{DeviceState, DeviceStateTransition, StaleDevice},
};

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
    let transaction = match database.orm().begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            record_failure(state, &error, "failed to start timeout transaction");
            return;
        }
    };
    let batch_size = i64::from(state.config.workers.device_heartbeat_batch_size);
    // LATERAL last-seen / target-state + SKIP LOCKED — irreducible.
    let stale_devices = match StaleDevice::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT device."deviceId" AS device_id,
              COALESCE(device.state::text, 'unknown') AS previous_state,
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
             AND COALESCE(device.state::text, 'unknown') <> target.target_state
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
        [batch_size.into()],
    ))
    .all(&transaction)
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
    let now = Utc::now();
    for device_row in stale_devices {
        let (device_state, change_state, alert_type, title, message, state_label) =
            if device_row.target_state == "faulty" {
                (
                    device::DeviceState::Faulty,
                    device_state_change::DeviceStateChangeState::Faulty,
                    alert::AlertType::Error,
                    "Device goes faulty".to_owned(),
                    format!("Device {} is faulty", device_row.device_id),
                    "faulty",
                )
            } else {
                (
                    device::DeviceState::NotReachable,
                    device_state_change::DeviceStateChangeState::NotReachable,
                    alert::AlertType::Warning,
                    format!("Device {} is not_reachable", device_row.device_id),
                    format!("Device {} is not_reachable", device_row.device_id),
                    "not_reachable",
                )
            };
        if let Err(error) = device::Entity::update_many()
            .col_expr(device::Column::State, Expr::value(Some(device_state)))
            .col_expr(device::Column::StateUpdatedAt, Expr::value(Some(now)))
            .col_expr(device::Column::UpdatedAt, Expr::value(now))
            .filter(device::Column::DeviceId.eq(&device_row.device_id))
            .exec(&transaction)
            .await
        {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to update stale device");
            return;
        }
        if let Err(error) = (device_state_change::ActiveModel {
            device_id: Set(device_row.device_id.clone()),
            state: Set(change_state),
            changed_at: Set(now),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        })
        .insert(&transaction)
        .await
        {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to store device state history");
            return;
        }
        if let Err(error) = (heartbeat::ActiveModel {
            device_id: Set(device_row.device_id.clone()),
            timestamp: Set(now),
            timeout: Set(device_row.timeout),
            data: Set(serde_json::json!({"status":"disconnected"})),
            ..Default::default()
        })
        .insert(&transaction)
        .await
        {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to store disconnected heartbeat");
            return;
        }
        let alert = match (alert::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set("ADMIN".to_owned()),
            title: Set(title),
            message: Set(message),
            r#type: Set(alert_type),
            status: Set(alert::AlertStatus::Unread),
            is_read: Set(false),
            device_id: Set(Some(device_row.device_id.clone())),
            data: Set(Some(serde_json::json!({
                "deviceId": device_row.device_id,
                "previousState": device_row.previous_state,
                "state": state_label,
            }))),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        })
        .insert(&transaction)
        .await
        {
            Ok(alert) => device_alert_json(&alert),
            Err(error) => {
                let _ = transaction.rollback().await;
                record_failure(state, &error, "failed to store device timeout alert");
                return;
            }
        };
        transitions.push(DeviceStateTransition {
            device_id: device_row.device_id,
            state: DeviceState::from(&device_row.target_state),
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

fn device_alert_json(model: &alert::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "user_id": model.user_id,
        "title": model.title,
        "message": model.message,
        "type": match model.r#type {
            alert::AlertType::Info => "info",
            alert::AlertType::Success => "success",
            alert::AlertType::Warning => "warning",
            alert::AlertType::Error => "error",
        },
        "status": match model.status {
            alert::AlertStatus::Unread => "unread",
            alert::AlertStatus::Read => "read",
        },
        "is_read": model.is_read,
        "device_id": model.device_id,
        "data": model.data,
        "created_at": model.created_at,
        "updated_at": model.updated_at,
    })
}

fn record_failure(state: &AppState, error: &sea_orm::DbErr, message: &str) {
    state
        .metrics
        .record_worker_run("device_heartbeat_timeout", "failure");
    tracing::error!(%error, worker = "device_heartbeat_timeout", "{message}");
}

#[cfg(test)]
mod tests {
    use crate::{socket::helpers::transition_events, workers::types::DeviceState};

    use super::*;

    #[test]
    fn timeout_events_preserve_device_frontend_contracts() {
        let [state, alert] = transition_events(&DeviceStateTransition {
            device_id: "device-1".to_owned(),
            state: DeviceState::NotReachable,
            alert: serde_json::json!({"title": "Device device-1 is not_reachable"}),
        });
        assert_eq!(state.event, "device_state_update");
        assert_eq!(state.payload["deviceId"], "device-1");
        assert_eq!(
            state.payload["state"],
            DeviceState::NotReachable.to_string()
        );
        assert_eq!(alert.event, "alert");
        assert_eq!(alert.payload["subtype"], "device-alert");
    }
}
