use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use crate::{
    devices::EDGE_CONTROLLER_PORT,
    events::{ServerEvent, build_performance_events},
    state::AppState,
};

#[derive(Debug, sqlx::FromRow)]
struct StuckDevice {
    device_id: String,
    attempted_at: Option<DateTime<Utc>>,
    controller_ip: Option<String>,
    power_port: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct FailedTest {
    test_id: String,
    build_id: Option<String>,
    user_id: Option<String>,
}

enum WatchdogDecision {
    Reboot,
    Wait,
    Fail,
}

struct WatchdogTransition {
    reboot: Option<(String, String)>,
    events: Vec<ServerEvent>,
}

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
    let device_ids = match sqlx::query_scalar::<_, String>(
        r#"SELECT device."deviceId"
           FROM devices device
           WHERE device.status::text = 'approved'
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
           ORDER BY device."updatedAt" ASC
           LIMIT $2"#,
    )
    .bind(timeout_seconds)
    .bind(i64::from(state.config.workers.gen5_reboot_batch_size))
    .fetch_all(database.pool())
    .await
    {
        Ok(device_ids) => device_ids,
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

async fn cleanup_recovered(state: &AppState) -> Result<(), String> {
    let database = state.database.as_ref().expect("worker requires database");
    let timeout_seconds =
        i64::try_from(state.config.workers.gen5_reboot_timeout_seconds).unwrap_or(i64::MAX);
    sqlx::query(
        r#"DELETE FROM farmcontroller.gen5_reboot_attempts attempt
           WHERE NOT EXISTS (
             SELECT 1 FROM devices device
             WHERE device."deviceId" = attempt.device_id
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
    )
    .bind(timeout_seconds)
    .execute(database.pool())
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn process_device(
    state: &AppState,
    device_id: &str,
) -> Result<Option<WatchdogTransition>, String> {
    let database = state.database.as_ref().expect("worker requires database");
    let mut transaction = database
        .pool()
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    let timeout_seconds =
        i64::try_from(state.config.workers.gen5_reboot_timeout_seconds).unwrap_or(i64::MAX);
    let device = sqlx::query_as::<_, StuckDevice>(
        r#"SELECT device."deviceId" AS device_id, attempt.attempted_at,
              mapping.controller_ip, mapping.power_port
           FROM devices device
           LEFT JOIN farmcontroller.gen5_reboot_attempts attempt
             ON attempt.device_id = device."deviceId"
           LEFT JOIN LATERAL (
             SELECT controller."ipAddress" AS controller_ip,
                    entry.value ->> 'power' AS power_port
             FROM device_controllers controller
             CROSS JOIN LATERAL jsonb_each(COALESCE(controller.mappings, '{}'::jsonb)) entry
             WHERE controller.status::text = 'approved'
               AND controller.state::text = 'active'
               AND controller."deletedAt" IS NULL
               AND regexp_replace(lower(entry.key), '[:-]', '', 'g') =
                   regexp_replace(lower(device."macAddress"), '[:-]', '', 'g')
               AND COALESCE(entry.value ->> 'power', '') <> ''
             ORDER BY controller."updatedAt" DESC LIMIT 1
           ) mapping ON true
           WHERE device."deviceId" = $1
             AND device.status::text = 'approved'
             AND replace(lower(device."deviceFamily"), ' ', '') LIKE '%gen5%'
             AND (device.upgrading = true OR device.flashing = true)
             AND device."updatedAt" < now() - ($2 * interval '1 second')
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
           FOR UPDATE OF device SKIP LOCKED"#,
    )
    .bind(device_id)
    .bind(timeout_seconds)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?;
    let Some(device) = device else {
        transaction
            .rollback()
            .await
            .map_err(|error| error.to_string())?;
        return Ok(None);
    };
    let timeout = chrono::Duration::seconds(timeout_seconds);
    let has_mapping = device.controller_ip.is_some() && device.power_port.is_some();
    match watchdog_decision(device.attempted_at, Utc::now(), timeout, has_mapping) {
        WatchdogDecision::Reboot => {
            let controller_ip = device.controller_ip.expect("mapping decision requires IP");
            let power_port = device.power_port.expect("mapping decision requires power");
            sqlx::query(
                r#"INSERT INTO farmcontroller.gen5_reboot_attempts
                       (device_id, attempted_at, controller_ip, power_port, created_at, updated_at)
                   VALUES ($1, now(), $2, $3, now(), now())
                   ON CONFLICT (device_id) DO NOTHING"#,
            )
            .bind(device_id)
            .bind(&controller_ip)
            .bind(&power_port)
            .execute(&mut *transaction)
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
            let events = fail_stuck_device(&mut transaction, &device).await?;
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

async fn fail_stuck_device(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    device: &StuckDevice,
) -> Result<Vec<ServerEvent>, String> {
    let reason = format!(
        "Unable to boot device - Gen5 device {} stuck after reboot watchdog timeout",
        device.device_id
    );
    let tests = sqlx::query_as::<_, FailedTest>(
        r#"SELECT DISTINCT test."testId" AS test_id, test."buildId"::text AS build_id,
                  COALESCE(test."createdBy", action."userId") AS user_id
           FROM test_executions test
           LEFT JOIN device_action_queue action
             ON action."testId" = test."testId"
            AND action."targetDeviceId" = $1
            AND action.status::text IN ('running', 'waiting')
           LEFT JOIN fallback_updates fallback
             ON fallback."testId" = test."testId"
            AND fallback."deviceId" = $1
            AND fallback.status::text IN ('pending', 'flashing')
           WHERE action.id IS NOT NULL OR fallback.id IS NOT NULL"#,
    )
    .bind(&device.device_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| error.to_string())?;
    sqlx::query(
        r#"UPDATE device_action_queue SET status = 'cancelled', "updatedAt" = now()
           WHERE "targetDeviceId" = $1 AND status::text IN ('running', 'waiting')"#,
    )
    .bind(&device.device_id)
    .execute(&mut **transaction)
    .await
    .map_err(|error| error.to_string())?;
    sqlx::query(
        r#"UPDATE fallback_updates SET status = 'failed', "updatedAt" = now()
           WHERE "deviceId" = $1 AND status::text IN ('pending', 'flashing')"#,
    )
    .bind(&device.device_id)
    .execute(&mut **transaction)
    .await
    .map_err(|error| error.to_string())?;
    for test in &tests {
        let changed = sqlx::query(
            r#"UPDATE test_executions
               SET status = 'failed', "executionPhase" = 'COMPLETE_FAILED',
                   "endedAt" = now(), "updatedAt" = now()
               WHERE "testId" = $1
                 AND status::text NOT IN ('completed', 'cancelled', 'failed')"#,
        )
        .bind(&test.test_id)
        .execute(&mut **transaction)
        .await
        .map_err(|error| error.to_string())?
        .rows_affected();
        if changed > 0 {
            sqlx::query(
                r#"INSERT INTO log_entries
                       (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                   VALUES ('test', $1, jsonb_build_object('message', $2), 'error', now(), now(), now())"#,
            )
            .bind(&test.test_id)
            .bind(&reason)
            .execute(&mut **transaction)
            .await
            .map_err(|error| error.to_string())?;
        }
    }
    sqlx::query(
        r#"UPDATE devices
           SET state = 'free', upgrading = false, flashing = false,
               "stateUpdatedAt" = now(), "updatedAt" = now()
           WHERE "deviceId" = $1"#,
    )
    .bind(&device.device_id)
    .execute(&mut **transaction)
    .await
    .map_err(|error| error.to_string())?;
    sqlx::query(
        r#"INSERT INTO device_state_change
               ("deviceId", state, "changedAt", "createdAt", "updatedAt")
           VALUES ($1, 'free', now(), now(), now())"#,
    )
    .bind(&device.device_id)
    .execute(&mut **transaction)
    .await
    .map_err(|error| error.to_string())?;
    sqlx::query("DELETE FROM farmcontroller.gen5_reboot_attempts WHERE device_id = $1")
        .bind(&device.device_id)
        .execute(&mut **transaction)
        .await
        .map_err(|error| error.to_string())?;
    let mut events = vec![device_state_event(&device.device_id)];
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

fn device_state_event(device_id: &str) -> ServerEvent {
    ServerEvent {
        event: "device_state_update".to_owned(),
        payload: serde_json::json!({
            "deviceId": device_id, "state": "free", "upgrading": false,
            "flashing": false, "changedAt": Utc::now().to_rfc3339(),
        }),
        room: None,
    }
}

fn test_failure_events(test_id: &str, build_id: &str, user_id: &str) -> Vec<ServerEvent> {
    let mut events = vec![
        ServerEvent {
            event: "test_execution".to_owned(),
            payload: serde_json::json!({
                "testId": test_id, "type": "status",
                "data": {"status": "failed", "timestamp": Utc::now().to_rfc3339()},
            }),
            room: Some(format!("user:{user_id}")),
        },
        ServerEvent {
            event: "test_execution_update".to_owned(),
            payload: serde_json::json!({"testId": test_id, "buildId": build_id, "status": "failed"}),
            room: Some(format!("build:{build_id}")),
        },
    ];
    events.extend(build_performance_events(build_id));
    events
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
        let device = device_state_event("device-1");
        let tests = test_failure_events("test-1", "build-1", "user-1");
        assert_eq!(device.payload["state"], "free");
        assert_eq!(device.payload["upgrading"], false);
        assert_eq!(tests[0].room.as_deref(), Some("user:user-1"));
        assert_eq!(tests[1].payload["status"], "failed");
        assert_eq!(tests[1].room.as_deref(), Some("build:build-1"));
        assert_eq!(tests[2].room.as_deref(), Some("dashboard:admin"));
        assert_eq!(tests[3].room.as_deref(), Some("dashboard:user"));
        assert_eq!(tests[4].room.as_deref(), Some("build:build-1"));
    }
}
