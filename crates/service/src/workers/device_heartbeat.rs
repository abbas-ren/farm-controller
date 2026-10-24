//! Device heartbeat timeout worker (legacy `HeartbeatQueueService` timeout path).
//!
//! Transitions approved devices to `not_reachable` / `faulty` when heartbeats
//! stop, raises alerts, and publishes device state-change events.

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    TransactionTrait,
    sea_query::{Expr, LockBehavior, LockType},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    persistence::entity::{alert, device, device_state_change, heartbeat},
    socket::helpers::transition_events,
    state::AppState,
    workers::types::{DeviceState, DeviceStateTransition, StaleDevice},
};

/// Poll loop for device heartbeat timeouts (faulty / not_reachable).
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

/// Claim stale devices, apply state transitions, raise alerts, publish events.
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
    let batch_size = u64::from(state.config.workers.device_heartbeat_batch_size);
    let candidates = match device::Entity::find()
        .filter(device::Column::DeletedAt.is_null())
        .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
        .filter(Expr::cust(
            r#"COALESCE(
                 (SELECT max(heartbeat.timestamp)
                  FROM heartbeats heartbeat
                  WHERE heartbeat."deviceId" = "devices"."deviceId"
                    AND NOT (heartbeat.data @> '{"status":"disconnected"}'::jsonb)),
                 "devices"."lastConnectedOn",
                 "devices"."createdAt"
               ) < now() - (GREATEST(COALESCE("devices"."heartbeatTimer", 5), 1) * 6 * interval '1 second')"#,
        ))
        .filter(Expr::cust(
            r#"NOT EXISTS (
                 SELECT 1 FROM device_action_queue action
                 WHERE action."targetDeviceId" = "devices"."deviceId"
                   AND action.status::text IN ('queued', 'waiting', 'running')
               )"#,
        ))
        .filter(Expr::cust(
            r#"NOT EXISTS (
                 SELECT 1 FROM fallback_updates fallback
                 WHERE fallback."deviceId" = "devices"."deviceId"
                   AND fallback.status::text IN ('pending', 'flashing')
               )"#,
        ))
        .order_by_asc::<Expr>(
            Expr::cust(
                r#"COALESCE(
                     (SELECT max(heartbeat.timestamp)
                      FROM heartbeats heartbeat
                      WHERE heartbeat."deviceId" = "devices"."deviceId"
                        AND NOT (heartbeat.data @> '{"status":"disconnected"}'::jsonb)),
                     "devices"."lastConnectedOn",
                     "devices"."createdAt"
                   )"#,
            )
            .into(),
        )
        .limit(batch_size)
        // SKIP LOCKED: concurrent worker ticks must not claim the same row.
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
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

    let mut transitions = Vec::with_capacity(candidates.len());
    let now = Utc::now();
    for device_row in candidates {
        let last_seen = match latest_connected_seen(&transaction, &device_row).await {
            Ok(last_seen) => last_seen,
            Err(error) => {
                let _ = transaction.rollback().await;
                record_failure(state, &error, "failed to resolve device last-seen");
                return;
            }
        };
        let Some(stale) = classify_stale_device(&device_row, last_seen, now) else {
            continue;
        };
        let (device_state, change_state, alert_type, title, message, state_label) =
            if stale.target_state == "faulty" {
                (
                    device::DeviceState::Faulty,
                    device_state_change::DeviceStateChangeState::Faulty,
                    alert::AlertType::Error,
                    "Device goes faulty".to_owned(),
                    format!("Device {} is faulty", stale.device_id),
                    "faulty",
                )
            } else {
                (
                    device::DeviceState::NotReachable,
                    device_state_change::DeviceStateChangeState::NotReachable,
                    alert::AlertType::Warning,
                    format!("Device {} is not_reachable", stale.device_id),
                    format!("Device {} is not_reachable", stale.device_id),
                    "not_reachable",
                )
            };
        if let Err(error) = device::Entity::update_many()
            .col_expr(device::Column::State, crate::persistence::pg_enum::pg_enum(&device_state))
            .col_expr(device::Column::StateUpdatedAt, Expr::value(Some(now)))
            .col_expr(device::Column::UpdatedAt, Expr::value(now))
            .filter(device::Column::DeviceId.eq(&stale.device_id))
            .exec(&transaction)
            .await
        {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to update stale device");
            return;
        }
        if let Err(error) = (device_state_change::ActiveModel {
            device_id: Set(stale.device_id.clone()),
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
            device_id: Set(stale.device_id.clone()),
            timestamp: Set(now),
            timeout: Set(stale.timeout),
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
            device_id: Set(Some(stale.device_id.clone())),
            data: Set(Some(serde_json::json!({
                "deviceId": stale.device_id,
                "previousState": stale.previous_state,
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
            device_id: stale.device_id,
            state: DeviceState::from(&stale.target_state),
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

/// Latest interface `last_seen` used when classifying reachability.
async fn latest_connected_seen(
    transaction: &sea_orm::DatabaseTransaction,
    device_row: &device::Model,
) -> Result<DateTime<Utc>, sea_orm::DbErr> {
    let heartbeat_seen = heartbeat::Entity::find()
        .filter(heartbeat::Column::DeviceId.eq(&device_row.device_id))
        .filter(Expr::cust(
            r#"NOT (data @> '{"status":"disconnected"}'::jsonb)"#,
        ))
        .order_by_desc(heartbeat::Column::Timestamp)
        .select_only()
        .column(heartbeat::Column::Timestamp)
        .into_tuple::<DateTime<Utc>>()
        .one(transaction)
        .await?;
    Ok(heartbeat_seen.unwrap_or(device_row.last_connected_on))
}


/// Choose target state from heartbeat age vs FAULTY / NOT_REACHABLE thresholds.
fn classify_stale_device(
    device_row: &device::Model,
    last_seen: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<StaleDevice> {
    let timer = device_row.heartbeat_timer.max(1) as i64;
    // Legacy HeartbeatTimeouts: NOT_REACHABLE = 6× timer, FAULTY = 12× timer (seconds).
    let not_reachable_after = chrono::Duration::seconds(timer * 6);
    let faulty_after = chrono::Duration::seconds(timer * 12);
    if now.signed_duration_since(last_seen) < not_reachable_after {
        return None;
    }
    // Phase: free/busy → not_reachable → faulty as silence lengthens.
    let target_state = if now.signed_duration_since(last_seen) >= faulty_after {
        "faulty"
    } else {
        "not_reachable"
    };
    let previous_state = match device_row.state.as_ref() {
        Some(device::DeviceState::Busy) => "busy",
        Some(device::DeviceState::NotReachable) => "not_reachable",
        Some(device::DeviceState::Free) => "free",
        Some(device::DeviceState::Faulty) => "faulty",
        Some(device::DeviceState::Unknown) | None => "unknown",
    };
    if previous_state == target_state {
        return None;
    }
    let timeout = now
        .signed_duration_since(last_seen)
        .num_seconds()
        .clamp(0, i64::from(i32::MAX)) as i32;
    Some(StaleDevice {
        device_id: device_row.device_id.clone(),
        previous_state: previous_state.to_owned(),
        target_state: target_state.to_owned(),
        timeout,
    })
}

/// Serialize a device alert into the frontend WS payload shape.
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

/// Record heartbeat worker failure metrics.
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
    /// Unit test: heartbeat timeout event shapes match the UI.
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
