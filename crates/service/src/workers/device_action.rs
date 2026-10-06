use std::{path::PathBuf, sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    devices::{DEVICE_ACTION_HTTP_TIMEOUT, DEVICE_COMMAND_RETRY_DELAY, EDGE_CONTROLLER_PORT},
    events::{ServerEvent, build_performance_events},
    state::AppState,
};

use super::test_preconditions;

#[derive(Debug, sqlx::FromRow)]
struct QueuedAction {
    id: Uuid,
    test_id: Option<String>,
    release_id: Option<Uuid>,
    version: Option<String>,
    device_type: String,
    user_id: String,
    mode: String,
    target_device_id: Option<String>,
    ipl_flash_confirm: Option<bool>,
    build_id: Option<String>,
    build_version: Option<String>,
    created_by: Option<String>,
}

struct AssignedAction {
    action: QueuedAction,
    device_id: String,
    ip_address: String,
}

#[derive(Debug, sqlx::FromRow)]
struct Gen4IplTarget {
    controller_ip: String,
    relay_serial: String,
    channel_number: i32,
    device_mac: String,
    gpio: String,
    gpio_default_level: String,
    relay_default_level: String,
}

pub(super) async fn worker(state: Arc<AppState>, cancellation: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.device_action_poll_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_cycle(&state).await,
        }
    }
    tracing::info!(worker = "device_action", "worker stopped");
}

async fn run_cycle(state: &AppState) {
    for _ in 0..state.config.workers.device_action_batch_size {
        let assigned = match assign_next(state).await {
            Ok(Some(assigned)) => assigned,
            Ok(None) => break,
            Err(error) => {
                record_failure(state, &error, "failed to assign queued action");
                break;
            }
        };
        for event in assignment_events(&assigned) {
            let _ = state.event_publisher.publish(event);
        }
        match dispatch(state, &assigned).await {
            Ok(()) => {
                state.metrics.record_worker_items("device_action", 1);
                state.metrics.record_worker_run("device_action", "success");
            }
            Err(error) => {
                tracing::error!(%error, action_id = %assigned.action.id, worker = "device_action", "device dispatch failed");
                if let Ok(events) = fail_dispatch(state, &assigned, &error).await {
                    for event in events {
                        let _ = state.event_publisher.publish(event);
                    }
                }
                state.metrics.record_worker_run("device_action", "failure");
            }
        }
    }
}

async fn assign_next(state: &AppState) -> Result<Option<AssignedAction>, sqlx::Error> {
    let database = state.database.as_ref().expect("worker requires database");
    let mut transaction = database.pool().begin().await?;
    let action = sqlx::query_as::<_, QueuedAction>(
        r#"SELECT action.id, action."testId" AS test_id,
                  action."releaseId" AS release_id, action.version,
                  action."deviceType" AS device_type, action."userId" AS user_id,
                  action.mode::text AS mode, action."targetDeviceId" AS target_device_id,
                  action."iplFlashConfirm" AS ipl_flash_confirm,
                  test."buildId"::text AS build_id, test."buildVersion" AS build_version,
                  test."createdBy" AS created_by
           FROM device_action_queue action
           LEFT JOIN test_executions test ON test."testId" = action."testId"
                     LEFT JOIN releases release
                         ON release.id::text = COALESCE(action."releaseId"::text, test."buildId"::text)
           WHERE action.status::text = 'queued'
             AND (lower(action."deviceType") NOT LIKE '%x5h%'
                  OR action."iplFlashConfirm" = true)
             AND (action.type::text <> 'test'
                  OR (test.status::text = 'not_executed'
                      AND COALESCE(test."cancelRequested", false) = false
                      AND test."executionPhase" = 'QUEUE_TEST_EXECUTION'
                      AND COALESCE(release."isFaulty", false) = false
                      AND (release."buildType"::text <> 'official'
                           OR release.status::text = 'passed')))
           ORDER BY CASE WHEN action.mode::text = 'deviceSpecific' THEN 0 ELSE 1 END,
                    action."createdAt" ASC
           LIMIT 1 FOR UPDATE OF action SKIP LOCKED"#,
    )
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(action) = action else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let confirmed = action.ipl_flash_confirm == Some(true);
    let devices = if action.mode == "deviceSpecific" || confirmed {
        sqlx::query_as::<_, (String, String)>(
            r#"SELECT "deviceId", "ipAddress" FROM devices
               WHERE "deviceId" = $1 AND "deviceType" = $2
                 AND status::text = 'approved' AND "deletedAt" IS NULL
                                 AND "ipAddress" IS NOT NULL
                 AND (state::text = 'free' OR $3)
               FOR UPDATE SKIP LOCKED"#,
        )
        .bind(action.target_device_id.as_deref())
        .bind(&action.device_type)
        .bind(confirmed)
        .fetch_all(&mut *transaction)
        .await?
    } else {
        sqlx::query_as::<_, (String, String)>(
            r#"SELECT "deviceId", "ipAddress" FROM devices
               WHERE "deviceType" = $1 AND status::text = 'approved'
                                 AND state::text = 'free' AND "deletedAt" IS NULL
                                 AND "ipAddress" IS NOT NULL
               ORDER BY "lastConnectedOn" DESC, "createdAt" ASC
               FOR UPDATE SKIP LOCKED"#,
        )
        .bind(&action.device_type)
        .fetch_all(&mut *transaction)
        .await?
    };
    let mut selected = None;
    for (device_id, ip_address) in devices {
        if let Some(test_id) = action.test_id.as_deref()
            && !confirmed
            && !test_preconditions::device_satisfies(&mut transaction, &device_id, test_id).await?
        {
            continue;
        }
        selected = Some((device_id, ip_address));
        break;
    }
    let Some((device_id, ip_address)) = selected else {
        transaction.rollback().await?;
        return Ok(None);
    };
    sqlx::query(
        r#"UPDATE device_action_queue
           SET status = 'running', "targetDeviceId" = $2, "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(action.id)
    .bind(&device_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"UPDATE devices SET state = 'busy', "lastTestExecution" = $2,
                            "lastExecutionStatus" = CASE WHEN $2::text IS NULL
                                THEN "lastExecutionStatus" ELSE 'in_progress' END,
                            "stateUpdatedAt" = now(), "updatedAt" = now()
           WHERE "deviceId" = $1"#,
    )
    .bind(&device_id)
    .bind(action.test_id.as_deref())
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"INSERT INTO device_state_change
              ("deviceId", state, "changedAt", "createdAt", "updatedAt")
           VALUES ($1, 'busy', now(), now(), now())"#,
    )
    .bind(&device_id)
    .execute(&mut *transaction)
    .await?;
    if let Some(test_id) = action.test_id.as_deref() {
        sqlx::query(
            r#"UPDATE test_executions SET status = 'in_progress',
                       "executionPhase" = 'DEVICE_ASSIGNED', "deviceId" = $2,
                       "startedAt" = COALESCE("startedAt", now()), "updatedAt" = now()
               WHERE "testId" = $1"#,
        )
        .bind(test_id)
        .bind(&device_id)
        .execute(&mut *transaction)
        .await?;
    } else if let Some(release_id) = action.release_id {
        sqlx::query(
            r#"UPDATE releases SET status = 'testing', "testedDeviceId" = $2,
                                  "updatedAt" = now()
               WHERE id = $1"#,
        )
        .bind(release_id)
        .bind(&device_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Some(AssignedAction {
        action,
        device_id,
        ip_address,
    }))
}

async fn dispatch(state: &AppState, assigned: &AssignedAction) -> Result<(), String> {
    if assigned.action.ipl_flash_confirm.is_none()
        && is_gen4_device(state, &assigned.device_id).await?
    {
        let target = resolve_gen4_ipl_target(state, &assigned.device_id)
            .await?
            .ok_or_else(|| {
                "Gen4 device requires a relay channel, controller mapping, and GPIO configuration"
                    .to_owned()
            })?;
        request_gen4_ipl(state, assigned, target).await?;
        return Ok(());
    }

    let (path, payload) = if let Some(test_id) = assigned.action.test_id.as_deref() {
        let build_version = assigned.action.build_version.as_deref().unwrap_or_default();
        let build_path = format!("{}/{test_id}", assigned.action.device_type);
        let current_test = PathBuf::from(&state.config.device.nfs_host_path)
            .join(&build_path)
            .join("usr/src/currentTest.txt");
        if let Some(parent) = current_test.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| error.to_string())?;
        }
        tokio::fs::write(current_test, test_id)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(database) = &state.database {
            sqlx::query(
                r#"UPDATE test_executions SET "executionPhase" = 'SEND_FLASH_REQUEST',
                                               "updatedAt" = now()
                   WHERE "testId" = $1"#,
            )
            .bind(test_id)
            .execute(database.pool())
            .await
            .map_err(|error| error.to_string())?;
        }
        (
            "test-execution",
            serde_json::json!({
                "nfs": format!("{}/{build_path}", state.config.device.nfs_host_path.trim_end_matches('/')),
                "server_ip": state.config.device.server_ip,
                "image": format!("{build_path}/Image"),
                "dtb": format!("{build_path}/board.dtb"),
                "testId": test_id,
                "build_ver": build_version,
                "deviceId": assigned.device_id,
            }),
        )
    } else {
        let version = assigned.action.version.as_deref().unwrap_or_default();
        let build_path = format!("{}/{version}", assigned.action.device_type);
        (
            "flash",
            serde_json::json!({
                "nfs": format!("{}/{build_path}", state.config.device.nfs_host_path.trim_end_matches('/')),
                "server_ip": state.config.device.server_ip,
                "image": format!("{build_path}/Image"),
                "dtb": format!("{build_path}/board.dtb"),
                "build_ver": version,
                "deviceId": assigned.device_id,
            }),
        )
    };
    let url = reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/{path}",
        assigned.ip_address
    ))
    .map_err(|error| error.to_string())?;
    let client = crate::external_http::client(Duration::from_secs(
        state.config.device.request_timeout_seconds,
    ))
    .map_err(|error| error.to_string())?;
    let mut last_error = String::new();
    for _ in 0..crate::external_http::EXPLICIT_COMMAND_ATTEMPTS {
        match client.post(url.clone()).json(&payload).send().await {
            Ok(response) if response.status().is_success() => {
                if let Some(test_id) = assigned.action.test_id.as_deref()
                    && let Some(database) = &state.database
                {
                    sqlx::query(
                        r#"UPDATE test_executions SET "executionPhase" = 'WAIT_FOR_DEVICE_BOOT',
                                                       "updatedAt" = now()
                           WHERE "testId" = $1"#,
                    )
                    .bind(test_id)
                    .execute(database.pool())
                    .await
                    .map_err(|error| error.to_string())?;
                }
                return Ok(());
            }
            Ok(response) => last_error = format!("device returned {}", response.status()),
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(DEVICE_COMMAND_RETRY_DELAY).await;
    }
    Err(last_error)
}

async fn is_gen4_device(state: &AppState, device_id: &str) -> Result<bool, String> {
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable for Gen4 IPL".to_owned())?;
    sqlx::query_scalar::<_, bool>(
        r#"SELECT replace(lower(COALESCE("deviceFamily", '')), ' ', '') = 'gen4'
           FROM devices
           WHERE "deviceId" = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(device_id)
    .fetch_optional(database.pool())
    .await
    .map(|value| value.unwrap_or(false))
    .map_err(|error| error.to_string())
}

async fn resolve_gen4_ipl_target(
    state: &AppState,
    device_id: &str,
) -> Result<Option<Gen4IplTarget>, String> {
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable for Gen4 IPL".to_owned())?;
    sqlx::query_as::<_, Gen4IplTarget>(
        r#"SELECT dc."ipAddress" AS controller_ip,
             r."serialNumber" AS relay_serial, rc."channelNumber" AS channel_number,
             COALESCE(d."macAddress", d."deviceId") AS device_mac,
             mapping.value ->> 'gpio' AS gpio,
             COALESCE(mapping.value ->> 'gpioDefaultLevel', 'LOW') AS gpio_default_level,
             COALESCE(mapping.value ->> 'relayDefaultLevel', 'LOW') AS relay_default_level
           FROM devices d
           JOIN relay_channels rc ON rc."deviceId" = d."deviceId" AND rc."deletedAt" IS NULL
           JOIN relays r ON r.id = rc."relayId" AND r."deletedAt" IS NULL
           JOIN device_controllers dc
             ON dc."deviceControllerId" = r."deviceControllerId" AND dc."deletedAt" IS NULL
           JOIN LATERAL jsonb_each(COALESCE(dc.mappings, '{}'::jsonb)) mapping
             ON regexp_replace(lower(mapping.key), '[:-]', '', 'g') =
                regexp_replace(lower(COALESCE(d."macAddress", d."deviceId")), '[:-]', '', 'g')
           WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL
             AND replace(lower(COALESCE(d."deviceFamily", '')), ' ', '') = 'gen4'
           ORDER BY rc."updatedAt" DESC LIMIT 1"#,
    )
    .bind(device_id)
    .fetch_optional(database.pool())
    .await
    .map_err(|error| error.to_string())
}

async fn request_gen4_ipl(
    state: &AppState,
    assigned: &AssignedAction,
    target: Gen4IplTarget,
) -> Result<(), String> {
    let gpio = target
        .gpio
        .parse::<u32>()
        .map_err(|_| "configured Gen4 GPIO is not numeric".to_owned())?;
    if !(0..=7).contains(&target.channel_number) {
        return Err("configured Gen4 relay channel is outside 0..7".to_owned());
    }
    for level in [&target.gpio_default_level, &target.relay_default_level] {
        if !matches!(level.as_str(), "HIGH" | "LOW") {
            return Err("configured Gen4 voltage level must be HIGH or LOW".to_owned());
        }
    }
    let mode_payload = serde_json::json!({
        "gen": 4,
        "gpio": gpio,
        "gpioDefaultLevel": target.gpio_default_level,
        "relayDefaultLevel": target.relay_default_level,
        "mac": target.device_mac.replace([':', '-'], "").to_lowercase(),
        "serial": target.relay_serial,
        "channel": target.channel_number,
    });
    let base = format!("http://{}:{EDGE_CONTROLLER_PORT}", target.controller_ip);
    let client = crate::external_http::client(DEVICE_ACTION_HTTP_TIMEOUT)
        .map_err(|error| error.to_string())?;
    let mode_response = client
        .post(format!("{base}/ipl-mode"))
        .json(&mode_payload)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !mode_response.status().is_success() {
        return Err(format!(
            "EdgeController rejected Gen4 IPL mode with {}",
            mode_response.status()
        ));
    }
    let version = assigned
        .action
        .build_version
        .as_deref()
        .or(assigned.action.version.as_deref())
        .filter(|version| !version.is_empty())
        .ok_or_else(|| "Gen4 IPL build version is missing".to_owned())?;
    let mut ipl_payload = mode_payload;
    ipl_payload["path"] = serde_json::Value::String(format!(
        "{}/{}",
        state
            .config
            .device
            .gen4_ipl_remote_path
            .trim_end_matches('/'),
        version
    ));
    let ipl_response = client
        .post(format!("{base}/ipl"))
        .json(&ipl_payload)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !ipl_response.status().is_success() {
        return Err(format!(
            "EdgeController rejected Gen4 IPL request with {}",
            ipl_response.status()
        ));
    }
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable for Gen4 IPL".to_owned())?;
    let mut transaction = database
        .pool()
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query(
        r#"UPDATE device_action_queue SET status = 'waiting', "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(assigned.action.id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?;
    if let Some(test_id) = assigned.action.test_id.as_deref() {
        sqlx::query(
            r#"UPDATE test_executions SET "executionPhase" = 'WAIT_FOR_IPL_CONFIRMATION',
                   "updatedAt" = now() WHERE "testId" = $1"#,
        )
        .bind(test_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
    }
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())
}

async fn fail_dispatch(
    state: &AppState,
    assigned: &AssignedAction,
    error: &str,
) -> Result<Vec<ServerEvent>, sqlx::Error> {
    let database = state.database.as_ref().expect("worker requires database");
    let mut transaction = database.pool().begin().await?;
    sqlx::query(
        r#"UPDATE device_action_queue SET status = 'cancelled', "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(assigned.action.id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"UPDATE devices SET state = 'free', "stateUpdatedAt" = now(), "updatedAt" = now()
           WHERE "deviceId" = $1"#,
    )
    .bind(&assigned.device_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"INSERT INTO device_state_change
              ("deviceId", state, "changedAt", "createdAt", "updatedAt")
           VALUES ($1, 'free', now(), now(), now())"#,
    )
    .bind(&assigned.device_id)
    .execute(&mut *transaction)
    .await?;
    if let Some(test_id) = assigned.action.test_id.as_deref() {
        sqlx::query(
            r#"UPDATE test_executions SET status = 'failed',
                       "executionPhase" = 'COMPLETE_FAILED', "endedAt" = now(),
                       "updatedAt" = now() WHERE "testId" = $1"#,
        )
        .bind(test_id)
        .execute(&mut *transaction)
        .await?;
    } else if let Some(release_id) = assigned.action.release_id {
        sqlx::query(
            r#"UPDATE releases SET status = 'failed', "isFaulty" = true, "updatedAt" = now()
               WHERE id = $1"#,
        )
        .bind(release_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(dispatch_failure_events(assigned, error))
}

fn assignment_events(assigned: &AssignedAction) -> Vec<ServerEvent> {
    let mut events = vec![device_state_event(&assigned.device_id, "busy")];
    if let Some(test_id) = assigned.action.test_id.as_deref() {
        let build_id = assigned.action.build_id.as_deref().unwrap_or_default();
        let owner = assigned
            .action
            .created_by
            .as_deref()
            .unwrap_or(&assigned.action.user_id);
        events.push(test_status_event(test_id, "in_progress", owner));
        events.push(test_build_event(test_id, build_id, "in_progress"));
        events.extend(build_performance_events(build_id));
    } else if let Some(release_id) = assigned.action.release_id {
        events.push(build_status_event(assigned, release_id, "testing", false));
    }
    events
}

fn dispatch_failure_events(assigned: &AssignedAction, error: &str) -> Vec<ServerEvent> {
    let mut events = vec![device_state_event(&assigned.device_id, "free")];
    if let Some(test_id) = assigned.action.test_id.as_deref() {
        let build_id = assigned.action.build_id.as_deref().unwrap_or_default();
        let owner = assigned
            .action
            .created_by
            .as_deref()
            .unwrap_or(&assigned.action.user_id);
        let mut status = test_status_event(test_id, "failed", owner);
        status.payload["data"]["message"] = serde_json::Value::String(error.to_owned());
        events.push(status);
        events.push(test_build_event(test_id, build_id, "failed"));
        events.extend(build_performance_events(build_id));
    } else if let Some(release_id) = assigned.action.release_id {
        events.push(build_status_event(assigned, release_id, "failed", true));
    }
    events
}

fn device_state_event(device_id: &str, state: &str) -> ServerEvent {
    ServerEvent {
        event: "device_state_update".to_owned(),
        payload: serde_json::json!({
            "deviceId": device_id, "state": state, "upgrading": false,
            "flashing": false, "changedAt": chrono::Utc::now().to_rfc3339(),
        }),
        room: None,
    }
}

fn test_status_event(test_id: &str, status: &str, user_id: &str) -> ServerEvent {
    ServerEvent {
        event: "test_execution".to_owned(),
        payload: serde_json::json!({
            "testId": test_id, "type": "status",
            "data": {"status": status, "timestamp": chrono::Utc::now().to_rfc3339()},
        }),
        room: Some(format!("user:{user_id}")),
    }
}

fn test_build_event(test_id: &str, build_id: &str, status: &str) -> ServerEvent {
    ServerEvent {
        event: "test_execution_update".to_owned(),
        payload: serde_json::json!({"testId": test_id, "buildId": build_id, "status": status}),
        room: Some(format!("build:{build_id}")),
    }
}

fn build_status_event(
    assigned: &AssignedAction,
    release_id: Uuid,
    status: &str,
    is_faulty: bool,
) -> ServerEvent {
    ServerEvent {
        event: "build_status_changed".to_owned(),
        payload: serde_json::json!({
            "releaseId": release_id, "status": status, "isFaulty": is_faulty,
            "version": assigned.action.version, "deviceType": assigned.action.device_type,
            "deviceId": assigned.device_id, "userId": assigned.action.user_id,
        }),
        room: None,
    }
}

fn record_failure(state: &AppState, error: &sqlx::Error, message: &str) {
    state.metrics.record_worker_run("device_action", "failure");
    tracing::error!(%error, worker = "device_action", "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assigned_test() -> AssignedAction {
        AssignedAction {
            action: QueuedAction {
                id: Uuid::nil(),
                test_id: Some("test-1".to_owned()),
                release_id: Some(Uuid::nil()),
                version: None,
                device_type: "x4h".to_owned(),
                user_id: "user-1".to_owned(),
                mode: "deviceType".to_owned(),
                target_device_id: None,
                ipl_flash_confirm: None,
                build_id: Some("build-1".to_owned()),
                build_version: Some("1.0.0".to_owned()),
                created_by: Some("user-1".to_owned()),
            },
            device_id: "device-1".to_owned(),
            ip_address: "192.0.2.1".to_owned(),
        }
    }

    #[test]
    fn assignment_events_refresh_device_owner_and_build_views() {
        let events = assignment_events(&assigned_test());
        assert_eq!(
            events
                .iter()
                .map(|event| event.event.as_str())
                .collect::<Vec<_>>(),
            vec![
                "device_state_update",
                "test_execution",
                "test_execution_update",
                "build_performance_update",
                "build_performance_update",
                "build_performance_update"
            ]
        );
        assert_eq!(events[1].payload["data"]["status"], "in_progress");
        assert_eq!(events[1].room.as_deref(), Some("user:user-1"));
        assert_eq!(events[2].room.as_deref(), Some("build:build-1"));
        assert_eq!(events[3].room.as_deref(), Some("dashboard:admin"));
        assert_eq!(events[4].room.as_deref(), Some("dashboard:user"));
        assert_eq!(events[5].room.as_deref(), Some("build:build-1"));
    }

    #[test]
    fn dispatch_failure_events_restore_device_and_fail_test() {
        let events = dispatch_failure_events(&assigned_test(), "connection refused");
        assert_eq!(events[0].payload["state"], "free");
        assert_eq!(events[1].payload["data"]["status"], "failed");
        assert_eq!(events[1].payload["data"]["message"], "connection refused");
    }
}
