use std::{collections::HashSet, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use crate::{
    devices::EDGE_CONTROLLER_PORT,
    persistence::entity::{
        device, device_action_queue, device_controller, device_state_change, fallback_update,
        gen5_reboot_attempt, log_entry, test_execution,
    },
    socket::{
        helpers::{device_state_event, test_failure_events},
        types::farm::ServerEvent,
    },
    state::AppState,
    workers::types::{DeviceState, FailedTest, StuckDevice, WatchdogDecision, WatchdogTransition},
};

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, LockBehavior, LockType},
};

pub(super) async fn worker(state: Arc<AppState>, cancellation: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.gen5_reboot_poll_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_cycle(&state).await,
        }
    }
    tracing::info!(worker = "gen5_reboot_watchdog", "worker stopped");
}

async fn run_cycle(state: &AppState) {
    let Some(database) = &state.database else {
        return;
    };
    if let Err(error) = cleanup_recovered(state).await {
        record_failure(state, &error, "failed to clean recovered reboot attempts");
        return;
    }
    let timeout_seconds =
        i64::try_from(state.config.workers.gen5_reboot_timeout_seconds).unwrap_or(i64::MAX);
    let batch_size = u64::from(state.config.workers.gen5_reboot_batch_size);
    let device_ids = match find_stuck_device_ids(database.orm(), timeout_seconds, batch_size).await
    {
        Ok(ids) => ids,
        Err(error) => {
            record_failure(
                state,
                &error.to_string(),
                "failed to find stuck Gen5 devices",
            );
            return;
        }
    };
    let mut processed = 0_u64;
    for device_id in device_ids {
        match process_device(state, &device_id).await {
            Ok(Some(transition)) => {
                if let Some((controller_ip, power_port)) = transition.reboot
                    && let Err(error) = send_reboot(state, &controller_ip, &power_port).await
                {
                    tracing::warn!(%error, %device_id, "Gen5 watchdog reboot request failed");
                }
                for event in transition.events {
                    let _ = state.event_publisher.publish(event);
                }
                processed += 1;
            }
            Ok(None) => {}
            Err(error) => record_failure(state, &error, "failed to process stuck Gen5 device"),
        }
    }
    state
        .metrics
        .record_worker_items("gen5_reboot_watchdog", processed);
    state
        .metrics
        .record_worker_run("gen5_reboot_watchdog", "success");
}

async fn find_stuck_device_ids(
    orm: &sea_orm::DatabaseConnection,
    timeout_seconds: i64,
    batch_size: u64,
) -> Result<Vec<String>, sea_orm::DbErr> {
    let rows = device::Entity::find()
        .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
        .filter(device::Column::DeletedAt.is_null())
        .filter(Expr::cust(
            r#"replace(lower("deviceFamily"), ' ', '') LIKE '%gen5%'"#,
        ))
        .filter(
            sea_orm::Condition::any()
                .add(device::Column::Upgrading.eq(true))
                .add(device::Column::Flashing.eq(true)),
        )
        .filter(Expr::cust_with_values(
            r#""updatedAt" < now() - ($1 * interval '1 second')"#,
            [timeout_seconds],
        ))
        .filter(Expr::cust(
            r#"(EXISTS (
                  SELECT 1 FROM device_action_queue action
                  WHERE action."targetDeviceId" = "devices"."deviceId"
                    AND action.status::text IN ('running', 'waiting')
                ) OR EXISTS (
                  SELECT 1 FROM fallback_updates fallback
                  WHERE fallback."deviceId" = "devices"."deviceId"
                    AND fallback.status::text IN ('pending', 'flashing')
                ))"#,
        ))
        .order_by_asc(device::Column::UpdatedAt)
        .limit(batch_size)
        .select_only()
        .column(device::Column::DeviceId)
        .into_tuple::<String>()
        .all(orm)
        .await?;
    Ok(rows)
}

async fn cleanup_recovered(state: &AppState) -> Result<(), String> {
    let database = state.database.as_ref().expect("worker requires database");
    let timeout_seconds =
        i64::try_from(state.config.workers.gen5_reboot_timeout_seconds).unwrap_or(i64::MAX);
    gen5_reboot_attempt::Entity::delete_many()
        .filter(Expr::cust_with_values(
            r#"NOT EXISTS (
                 SELECT 1 FROM devices device
                 WHERE device."deviceId" = device_id
                   AND device.status::text = 'approved'
                   AND replace(lower(device."deviceFamily"), ' ', '') LIKE '%gen5%'
                   AND (device.upgrading = true OR device.flashing = true)
                   AND device."updatedAt" < now() - ($1 * interval '1 second')
                   AND device."deletedAt" IS NULL
                   AND (EXISTS (
                          SELECT 1 FROM device_action_queue action
                          WHERE action."targetDeviceId" = device."deviceId"
                            AND action.status::text IN ('running', 'waiting')
                        ) OR EXISTS (
                          SELECT 1 FROM fallback_updates fallback
                          WHERE fallback."deviceId" = device."deviceId"
                            AND fallback.status::text IN ('pending', 'flashing')
                        ))
               )"#,
            [timeout_seconds],
        ))
        .exec(database.orm())
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn process_device(
    state: &AppState,
    device_id: &str,
) -> Result<Option<WatchdogTransition>, String> {
    let database = state.database.as_ref().expect("worker requires database");
    let transaction = database
        .orm()
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    let timeout_seconds =
        i64::try_from(state.config.workers.gen5_reboot_timeout_seconds).unwrap_or(i64::MAX);
    let locked = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
        .filter(device::Column::DeletedAt.is_null())
        .filter(Expr::cust(
            r#"replace(lower("deviceFamily"), ' ', '') LIKE '%gen5%'"#,
        ))
        .filter(
            sea_orm::Condition::any()
                .add(device::Column::Upgrading.eq(true))
                .add(device::Column::Flashing.eq(true)),
        )
        .filter(Expr::cust_with_values(
            r#""updatedAt" < now() - ($1 * interval '1 second')"#,
            [timeout_seconds],
        ))
        .filter(Expr::cust(
            r#"(EXISTS (
                  SELECT 1 FROM device_action_queue action
                  WHERE action."targetDeviceId" = "devices"."deviceId"
                    AND action.status::text IN ('running', 'waiting')
                ) OR EXISTS (
                  SELECT 1 FROM fallback_updates fallback
                  WHERE fallback."deviceId" = "devices"."deviceId"
                    AND fallback.status::text IN ('pending', 'flashing')
                ))"#,
        ))
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .one(&transaction)
        .await
        .map_err(|error| error.to_string())?;
    let Some(locked) = locked else {
        transaction
            .rollback()
            .await
            .map_err(|error| error.to_string())?;
        return Ok(None);
    };
    let attempted_at = gen5_reboot_attempt::Entity::find_by_id(&locked.device_id)
        .one(&transaction)
        .await
        .map_err(|error| error.to_string())?
        .map(|attempt| attempt.attempted_at);
    let (controller_ip, power_port) = resolve_power_mapping(&transaction, &locked)
        .await
        .map_err(|error| error.to_string())?;
    let device = StuckDevice {
        device_id: locked.device_id,
        attempted_at,
        controller_ip,
        power_port,
    };
    let timeout = chrono::Duration::seconds(timeout_seconds);
    let has_mapping = device.controller_ip.is_some() && device.power_port.is_some();
    match watchdog_decision(device.attempted_at, Utc::now(), timeout, has_mapping) {
        WatchdogDecision::Reboot => {
            let controller_ip = device.controller_ip.expect("mapping decision requires IP");
            let power_port = device.power_port.expect("mapping decision requires power");
            let now = Utc::now();
            gen5_reboot_attempt::Entity::insert(gen5_reboot_attempt::ActiveModel {
                device_id: Set(device_id.to_owned()),
                attempted_at: Set(now),
                controller_ip: Set(controller_ip.clone()),
                power_port: Set(power_port.clone()),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .on_conflict_do_nothing()
            .exec(&transaction)
            .await
            .map_err(|error| error.to_string())?;
            transaction
                .commit()
                .await
                .map_err(|error| error.to_string())?;
            Ok(Some(WatchdogTransition {
                reboot: Some((controller_ip, power_port)),
                events: Vec::new(),
            }))
        }
        WatchdogDecision::Wait => {
            transaction
                .commit()
                .await
                .map_err(|error| error.to_string())?;
            Ok(None)
        }
        WatchdogDecision::Fail => {
            let events = fail_stuck_device(&transaction, &device).await?;
            transaction
                .commit()
                .await
                .map_err(|error| error.to_string())?;
            Ok(Some(WatchdogTransition {
                reboot: None,
                events,
            }))
        }
    }
}

async fn resolve_power_mapping(
    transaction: &impl ConnectionTrait,
    device: &device::Model,
) -> Result<(Option<String>, Option<String>), sea_orm::DbErr> {
    let normalized_mac = normalize_mac(device.mac_address.as_deref().unwrap_or_default());
    if normalized_mac.is_empty() {
        return Ok((None, None));
    }
    let controllers = device_controller::Entity::find()
        .filter(device_controller::Column::Status.eq(device_controller::DeviceControllerStatus::Approved))
        .filter(
            device_controller::Column::State.eq(device_controller::DeviceControllerState::Active),
        )
        .filter(device_controller::Column::DeletedAt.is_null())
        .order_by_desc(device_controller::Column::UpdatedAt)
        .all(transaction)
        .await?;
    for controller in controllers {
        if let Some(mapping) = mapping_entry_for_mac(&controller.mappings, &normalized_mac) {
            let power = mapping
                .get("power")
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            if let Some(power_port) = power {
                return Ok((Some(controller.ip_address), Some(power_port)));
            }
        }
    }
    Ok((None, None))
}

fn normalize_mac(value: &str) -> String {
    value.to_ascii_lowercase().replace([':', '-'], "")
}

fn mapping_entry_for_mac<'a>(
    mappings: &'a serde_json::Value,
    normalized_mac: &str,
) -> Option<&'a serde_json::Value> {
    let object = mappings.as_object()?;
    object
        .iter()
        .find_map(|(key, value)| (normalize_mac(key) == normalized_mac).then_some(value))
}

async fn fail_stuck_device(
    transaction: &impl ConnectionTrait,
    device: &StuckDevice,
) -> Result<Vec<ServerEvent>, String> {
    let reason = format!(
        "Unable to boot device - Gen5 device {} stuck after reboot watchdog timeout",
        device.device_id
    );
    let action_tests = device_action_queue::Entity::find()
        .filter(device_action_queue::Column::TargetDeviceId.eq(&device.device_id))
        .filter(
            device_action_queue::Column::Status.is_in([
                device_action_queue::QueueJobStatus::Running,
                device_action_queue::QueueJobStatus::Waiting,
            ]),
        )
        .all(transaction)
        .await
        .map_err(|error| error.to_string())?;
    let fallback_tests = fallback_update::Entity::find()
        .filter(fallback_update::Column::DeviceId.eq(&device.device_id))
        .filter(
            fallback_update::Column::Status.is_in([
                fallback_update::FallbackStatus::Pending,
                fallback_update::FallbackStatus::Flashing,
            ]),
        )
        .all(transaction)
        .await
        .map_err(|error| error.to_string())?;
    let mut test_ids = HashSet::new();
    for action in &action_tests {
        if let Some(test_id) = &action.test_id {
            test_ids.insert(test_id.clone());
        }
    }
    for fallback in &fallback_tests {
        test_ids.insert(fallback.test_id.clone());
    }
    let mut tests = Vec::new();
    for test_id in test_ids {
        let Some(execution) = test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(&test_id))
            .one(transaction)
            .await
            .map_err(|error| error.to_string())?
        else {
            continue;
        };
        let user_id = execution.created_by.clone().or_else(|| {
            action_tests
                .iter()
                .find(|action| action.test_id.as_deref() == Some(test_id.as_str()))
                .map(|action| action.user_id.clone())
        });
        tests.push(FailedTest {
            test_id,
            build_id: Some(execution.build_id),
            user_id,
        });
    }
    let now = Utc::now();
    device_action_queue::Entity::update_many()
        .col_expr(
            device_action_queue::Column::Status,
            Expr::value(device_action_queue::QueueJobStatus::Cancelled),
        )
        .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
        .filter(device_action_queue::Column::TargetDeviceId.eq(&device.device_id))
        .filter(
            device_action_queue::Column::Status.is_in([
                device_action_queue::QueueJobStatus::Running,
                device_action_queue::QueueJobStatus::Waiting,
            ]),
        )
        .exec(transaction)
        .await
        .map_err(|error| error.to_string())?;
    fallback_update::Entity::update_many()
        .col_expr(
            fallback_update::Column::Status,
            Expr::value(fallback_update::FallbackStatus::Failed),
        )
        .col_expr(fallback_update::Column::UpdatedAt, Expr::value(now))
        .filter(fallback_update::Column::DeviceId.eq(&device.device_id))
        .filter(
            fallback_update::Column::Status.is_in([
                fallback_update::FallbackStatus::Pending,
                fallback_update::FallbackStatus::Flashing,
            ]),
        )
        .exec(transaction)
        .await
        .map_err(|error| error.to_string())?;
    for test in &tests {
        let changed = test_execution::Entity::update_many()
            .col_expr(
                test_execution::Column::Status,
                Expr::value(Some(test_execution::TestStatus::Failed)),
            )
            .col_expr(
                test_execution::Column::ExecutionPhase,
                Expr::value(Some(test_execution::ExecutionPhase::CompleteFailed)),
            )
            .col_expr(test_execution::Column::EndedAt, Expr::value(Some(now)))
            .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
            .filter(test_execution::Column::TestId.eq(&test.test_id))
            .filter(test_execution::Column::Status.is_not_in([
                test_execution::TestStatus::Completed,
                test_execution::TestStatus::Cancelled,
                test_execution::TestStatus::Failed,
            ]))
            .exec(transaction)
            .await
            .map_err(|error| error.to_string())?
            .rows_affected;
        if changed > 0 {
            log_entry::ActiveModel {
                r#type: Set(log_entry::LogEntryType::Test),
                reference_id: Set(test.test_id.clone()),
                data: Set(serde_json::json!({ "message": reason })),
                level: Set(log_entry::LogEntryLevel::Error),
                timestamp: Set(now),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(transaction)
            .await
            .map_err(|error| error.to_string())?;
        }
    }
    device::Entity::update_many()
        .col_expr(
            device::Column::State,
            Expr::value(Some(device::DeviceState::Free)),
        )
        .col_expr(device::Column::Upgrading, Expr::value(Some(false)))
        .col_expr(device::Column::Flashing, Expr::value(Some(false)))
        .col_expr(device::Column::StateUpdatedAt, Expr::value(Some(now)))
        .col_expr(device::Column::UpdatedAt, Expr::value(now))
        .filter(device::Column::DeviceId.eq(&device.device_id))
        .exec(transaction)
        .await
        .map_err(|error| error.to_string())?;
    device_state_change::ActiveModel {
        device_id: Set(device.device_id.clone()),
        state: Set(device_state_change::DeviceStateChangeState::Free),
        changed_at: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await
    .map_err(|error| error.to_string())?;
    gen5_reboot_attempt::Entity::delete_by_id(&device.device_id)
        .exec(transaction)
        .await
        .map_err(|error| error.to_string())?;
    let mut events = vec![device_state_event(&device.device_id, DeviceState::Free)];
    for test in tests {
        events.extend(test_failure_events(
            &test.test_id,
            test.build_id.as_deref().unwrap_or_default(),
            test.user_id.as_deref().unwrap_or_default(),
        ));
    }
    Ok(events)
}

async fn send_reboot(
    state: &AppState,
    controller_ip: &str,
    power_port: &str,
) -> Result<(), String> {
    let url = reqwest::Url::parse(&format!(
        "http://{controller_ip}:{EDGE_CONTROLLER_PORT}/reboot-device"
    ))
    .map_err(|error| error.to_string())?;
    crate::external_http::client(Duration::from_secs(
        state.config.workers.gen5_reboot_request_timeout_seconds,
    ))
    .map_err(|error| error.to_string())?
    .post(url)
    .json(&serde_json::json!({"power": power_port}))
    .send()
    .await
    .map_err(|error| error.to_string())?
    .error_for_status()
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn watchdog_decision(
    attempted_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    timeout: chrono::Duration,
    has_mapping: bool,
) -> WatchdogDecision {
    if !has_mapping {
        WatchdogDecision::Fail
    } else if let Some(attempted_at) = attempted_at {
        if now.signed_duration_since(attempted_at) >= timeout {
            WatchdogDecision::Fail
        } else {
            WatchdogDecision::Wait
        }
    } else {
        WatchdogDecision::Reboot
    }
}

fn record_failure(state: &AppState, error: &str, message: &str) {
    state
        .metrics
        .record_worker_run("gen5_reboot_watchdog", "failure");
    tracing::error!(%error, worker = "gen5_reboot_watchdog", "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_detection_reboots_then_waits_then_fails() {
        let now = Utc::now();
        let timeout = chrono::Duration::seconds(120);
        assert!(matches!(
            watchdog_decision(None, now, timeout, true),
            WatchdogDecision::Reboot
        ));
        assert!(matches!(
            watchdog_decision(
                Some(now),
                now + chrono::Duration::seconds(119),
                timeout,
                true
            ),
            WatchdogDecision::Wait
        ));
        assert!(matches!(
            watchdog_decision(Some(now), now + timeout, timeout, true),
            WatchdogDecision::Fail
        ));
        assert!(matches!(
            watchdog_decision(None, now, timeout, false),
            WatchdogDecision::Fail
        ));
    }

    #[test]
    fn watchdog_failure_events_refresh_device_owner_and_build() {
        let device = device_state_event("device-1", DeviceState::Free);
        let tests = test_failure_events("test-1", "build-1", "user-1");
        assert_eq!(device.payload["state"], DeviceState::Free.to_string());
        assert_eq!(device.payload["upgrading"], false);
        assert_eq!(tests[1].payload["status"], "failed");
    }
}
