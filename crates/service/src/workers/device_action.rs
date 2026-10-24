//! Device action queue dispatcher (legacy `DeviceActionQueueService`).
//!
//! Polls `device_action_queue` for eligible queued jobs, claims one under
//! `FOR UPDATE SKIP LOCKED`, assigns a free device, and dispatches flash/test/IPL
//! work to the edge controller. Publishes assignment and failure farm events.

use std::{path::PathBuf, sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    devices::{DEVICE_ACTION_HTTP_TIMEOUT, DEVICE_COMMAND_RETRY_DELAY, EDGE_CONTROLLER_PORT},
    persistence::entity::{
        device, device_action_queue, device_controller, device_state_change, relay, relay_channel,
        release, test_execution,
    },
    socket::{
        helpers::{assignment_events, dispatch_failure_events},
        types::farm::ServerEvent,
    },
    state::AppState,
    workers::types::{AssignedAction, Gen4IplTarget, QueuedAction, err_seaorm_to_sqlx},
};

use super::test_preconditions;

use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ExprTrait, JoinType, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait, RelationTrait, Set, TransactionTrait,
    sea_query::{Alias, Expr, LockBehavior, LockType},
};

/// Poll loop for the device-action queue until cancellation.
pub(super) async fn worker(state: Arc<AppState>, cancellation: CancellationToken) {
    info!(worker = "device_action", "worker started");
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

/// Claim and dispatch up to `device_action_batch_size` jobs in one tick.
///
/// Publishes assignment events, appends test logs, and records metrics per dispatch.
async fn run_cycle(state: &AppState) {
    for _ in 0..state.config.workers.device_action_batch_size {
        let assigned = match assign_next(state).await {
            Ok(Some(assigned)) => assigned,
            Ok(None) => {
                info!(worker = "device_action", "no queued actions available");
                break;
            }
            Err(err) => {
                error!(%err, worker = "device_action", "failed to assign queued action");
                record_failure(
                    state,
                    &err_seaorm_to_sqlx(err),
                    "failed to assign queued action",
                );
                break;
            }
        };
        for event in assignment_events(&assigned) {
            let _ = state.event_publisher.publish(event);
        }
        if let Some(test_id) = assigned.action.test_id.as_deref() {
            let message = if assigned.action.ipl_flash_confirm == Some(true) {
                format!(
                    "Resuming test execution on device {} after IPL confirmation",
                    assigned.device_id
                )
            } else {
                format!("Starting test execution on device {}", assigned.device_id)
            };
            if let Err(error) =
                crate::devices::test_log::append_test_log(state, test_id, &message).await
            {
                tracing::warn!(%error, %test_id, "failed to append assignment test log");
            }
        }
        info!(action_id = %assigned.action.id, worker = "device_action", "dispatching action");
        match dispatch(state, &assigned).await {
            Ok(()) => {
                state.metrics.record_worker_items("device_action", 1);
                state.metrics.record_worker_run("device_action", "success");
            }
            Err(error) => {
                tracing::error!(%error, action_id = %assigned.action.id, worker = "device_action", "device dispatch failed");
                if let Some(test_id) = assigned.action.test_id.as_deref() {
                    let message = format!("Device dispatch failed: {error}");
                    if let Err(log_error) =
                        crate::devices::test_log::append_test_log(state, test_id, &message).await
                    {
                        tracing::warn!(%log_error, %test_id, "failed to append dispatch failure log");
                    }
                }
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

/// Atomically claim the next eligible queued action and a free device.
///
/// Uses a transaction with `FOR UPDATE SKIP LOCKED` so concurrent workers do not
/// double-assign the same queue row or device. Returns `Ok(None)` when idle.
async fn assign_next(state: &AppState) -> Result<Option<AssignedAction>, sea_orm::DbErr> {
    let database = state.database.as_ref().expect("worker requires database");

    let transaction = database.orm().begin().await?;

    // ---------------------------------------------------------------
    // 1. Find and lock the next queued action.
    // ---------------------------------------------------------------

    let mut action_query = device_action_queue::Entity::find()
        .join(
            JoinType::LeftJoin,
            device_action_queue::Relation::TestExecution.def(),
        )
        .join(
            JoinType::LeftJoin,
            device_action_queue::Relation::Release.def(),
        )
        .select_only()
        .column(device_action_queue::Column::Id)
        .column(device_action_queue::Column::TestId)
        .column(device_action_queue::Column::ReleaseId)
        .column(device_action_queue::Column::Version)
        .column(device_action_queue::Column::DeviceType)
        .column(device_action_queue::Column::UserId)
        .column(device_action_queue::Column::Mode)
        .column(device_action_queue::Column::TargetDeviceId)
        .column(device_action_queue::Column::IplFlashConfirm)
        .column_as(test_execution::Column::BuildId, "build_id")
        .column_as(test_execution::Column::BuildVersion, "build_version")
        .column_as(test_execution::Column::CreatedBy, "created_by")
        // Claim filter: only QUEUED rows.
        .filter(device_action_queue::Column::Status.eq(device_action_queue::QueueJobStatus::Queued))
        // Claim filter: skip raw x5h types unless this is a post-IPL confirm requeue.
        .filter(
            sea_orm::Condition::any()
                .add(device_action_queue::Column::DeviceType.not_like("%x5h%"))
                .add(device_action_queue::Column::IplFlashConfirm.eq(true)),
        )
        .filter(
            sea_orm::Condition::any()
                .add(device_action_queue::Column::Type.ne(device_action_queue::QueueJobType::Test))
                .add(
                    sea_orm::Condition::all()
                        // First assignment: fresh job still waiting in queue.
                        // Post-IPL requeue: status is already in_progress and phase is
                        // REQUEUE_AFTER_IPL_CONFIRMATION (legacy allowed these via
                        // iplFlashConfirm alone).
                        .add(
                            sea_orm::Condition::any()
                                .add(
                                    sea_orm::Condition::all()
                                        .add(
                                            test_execution::Column::Status
                                                .eq(test_execution::TestStatus::NotExecuted),
                                        )
                                        .add(
                                            test_execution::Column::ExecutionPhase.eq(
                                                test_execution::ExecutionPhase::QueueTestExecution,
                                            ),
                                        ),
                                )
                                .add(
                                    sea_orm::Condition::all()
                                        .add(
                                            device_action_queue::Column::IplFlashConfirm.eq(true),
                                        )
                                        .add(
                                            test_execution::Column::Status
                                                .eq(test_execution::TestStatus::InProgress),
                                        )
                                        .add(
                                            test_execution::Column::ExecutionPhase.eq(
                                                test_execution::ExecutionPhase::RequeueAfterIplConfirmation,
                                            ),
                                        ),
                                ),
                        )
                        .add(
                            sea_orm::Condition::any()
                                .add(test_execution::Column::CancelRequested.eq(false))
                                .add(test_execution::Column::CancelRequested.is_null()),
                        )
                        .add(
                            sea_orm::Condition::any()
                                .add(release::Column::IsFaulty.eq(false))
                                .add(release::Column::IsFaulty.is_null()),
                        )
                        .add(
                            sea_orm::Condition::any()
                                .add(release::Column::BuildType.ne(release::BuildType::Official))
                                .add(
                                    release::Column::Status.eq(release::ReleaseTestStatus::Passed),
                                ),
                        ),
                ),
        )
        .order_by_asc::<Expr>(
            Expr::case(
                // Cast enum literal: plain text compare fails with enum_device_action_queue_mode = text.
                Expr::col(device_action_queue::Column::Mode).eq(
                    crate::persistence::pg_enum::pg_enum(&device_action_queue::QueueDispatchMode::DeviceSpecific),
                ),
                0,
            )
            .finally(1)
            .into(),
        )
        .order_by_asc(device_action_queue::Column::CreatedAt)
        .limit(1);
    // FOR UPDATE OF queue only — Postgres rejects locking the nullable LEFT JOIN side.
    QueryTrait::query(&mut action_query).lock_with_tables_behavior(
        LockType::Update,
        [Alias::new("device_action_queue")],
        LockBehavior::SkipLocked,
    );
    let action = action_query
        .into_model::<QueuedAction>()
        .one(&transaction)
        .await?;

    tracing::info!("Fetched action from queue");

    let Some(action) = action else {
        transaction.rollback().await?;
        return Ok(None);
    };

    let confirmed = action.ipl_flash_confirm == Some(true);

    // ---------------------------------------------------------------
    // 2. Find an eligible device.
    // ---------------------------------------------------------------

    let devices = if matches!(
        action.mode,
        device_action_queue::QueueDispatchMode::DeviceSpecific
    ) || confirmed
    {
        let Some(target_device_id) = action.target_device_id.as_deref() else {
            transaction.rollback().await?;
            return Ok(None);
        };

        let mut query = device::Entity::find()
            .filter(device::Column::DeviceId.eq(target_device_id))
            .filter(device::Column::DeviceType.eq(&action.device_type))
            .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
            .filter(device::Column::DeletedAt.is_null())
            .filter(device::Column::IpAddress.is_not_null());

        if !confirmed {
            query = query.filter(device::Column::State.eq(device::DeviceState::Free));
        }

        query
            .select_only()
            .column(device::Column::DeviceId)
            .column(device::Column::IpAddress)
            // SKIP LOCKED: concurrent worker ticks must not claim the same row.
            .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
            .into_tuple::<(String, Option<String>)>()
            .all(&transaction)
            .await?
    } else {
        device::Entity::find()
            .filter(device::Column::DeviceType.eq(&action.device_type))
            .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
            .filter(device::Column::State.eq(device::DeviceState::Free))
            .filter(device::Column::DeletedAt.is_null())
            .filter(device::Column::IpAddress.is_not_null())
            .order_by_desc(device::Column::LastConnectedOn)
            .order_by_asc(device::Column::CreatedAt)
            .select_only()
            .column(device::Column::DeviceId)
            .column(device::Column::IpAddress)
            // SKIP LOCKED: concurrent worker ticks must not claim the same row.
            .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
            .into_tuple::<(String, Option<String>)>()
            .all(&transaction)
            .await?
    };

    // ---------------------------------------------------------------
    // 3. Apply test-specific preconditions.
    // ---------------------------------------------------------------

    let mut selected = None;

    for (device_id, ip_address) in devices {
        let Some(ip_address) = ip_address else {
            continue;
        };

        if let Some(test_id) = action.test_id.as_deref()
            && !confirmed
            && !test_preconditions::device_satisfies(&transaction, &device_id, test_id).await?
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

    // ---------------------------------------------------------------
    // 4. Mark queue action as running.
    // ---------------------------------------------------------------

    let action_model = device_action_queue::ActiveModel {
        id: Set(action.id),
        status: Set(device_action_queue::QueueJobStatus::Running),
        target_device_id: Set(Some(device_id.clone())),
        updated_at: Set(chrono::Utc::now()),
        ..Default::default()
    };

    action_model.update(&transaction).await?;

    // ---------------------------------------------------------------
    // 5. Mark device as busy.
    // ---------------------------------------------------------------

    let device_model = device::Entity::find()
        .filter(device::Column::DeviceId.eq(&device_id))
        .one(&transaction)
        .await?
        .ok_or_else(|| sea_orm::DbErr::RecordNotFound(format!("device {device_id} disappeared")))?;

    let last_execution_status = if action.test_id.is_some() {
        Some(device::TestStatus::InProgress)
    } else {
        device_model.last_execution_status.clone()
    };

    let device_update = device::ActiveModel {
        id: Set(device_model.id),
        state: Set(Some(device::DeviceState::Busy)),
        last_test_execution: Set(action.test_id.clone()),
        last_execution_status: Set(last_execution_status),
        state_updated_at: Set(Some(chrono::Utc::now())),
        updated_at: Set(chrono::Utc::now()),
        ..Default::default()
    };

    device_update.update(&transaction).await?;

    // ---------------------------------------------------------------
    // 6. Insert device state change.
    // ---------------------------------------------------------------

    let now = chrono::Utc::now();

    device_state_change::ActiveModel {
        device_id: Set(device_id.clone()),
        state: Set(device_state_change::DeviceStateChangeState::Busy),
        changed_at: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&transaction)
    .await?;

    // ---------------------------------------------------------------
    // 7. Update test execution or release.
    // ---------------------------------------------------------------

    if let Some(test_id) = action.test_id.as_deref() {
        let test = test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(test_id))
            .one(&transaction)
            .await?
            .ok_or_else(|| {
                sea_orm::DbErr::RecordNotFound(format!("test execution {test_id} disappeared"))
            })?;

        test_execution::ActiveModel {
            id: Set(test.id),
            status: Set(Some(test_execution::TestStatus::InProgress)),
            execution_phase: Set(Some(test_execution::ExecutionPhase::DeviceAssigned)),
            device_id: Set(Some(device_id.clone())),
            started_at: Set(test.started_at.or(Some(now))),
            updated_at: Set(now),
            ..Default::default()
        }
        .update(&transaction)
        .await?;
    } else if let Some(release_id) = action.release_id {
        let release = release::Entity::find_by_id(release_id)
            .one(&transaction)
            .await?
            .ok_or_else(|| {
                sea_orm::DbErr::RecordNotFound(format!("release {release_id} disappeared"))
            })?;

        release::ActiveModel {
            id: Set(release.id),
            status: Set(Some(release::ReleaseTestStatus::Testing)),
            tested_device_id: Set(Some(device_id.clone())),
            updated_at: Set(now),
            ..Default::default()
        }
        .update(&transaction)
        .await?;
    }

    // ---------------------------------------------------------------
    // 8. Commit.
    // ---------------------------------------------------------------

    transaction.commit().await?;

    Ok(Some(AssignedAction {
        action,
        device_id,
        ip_address,
    }))
}

/// Send the assigned flash/test/IPL command to the edge controller.
///
/// Side effects: HTTP to the controller, possible Gen4 IPL relay GPIO sequence,
/// and queue/test status updates on success paths inside helpers.
async fn dispatch(state: &AppState, assigned: &AssignedAction) -> Result<(), String> {
    if assigned.action.ipl_flash_confirm.is_none()
        && is_gen4_device(state, &assigned.device_id).await?
    {
        let target = resolve_gen4_ipl_target(state, &assigned.device_id)
            .await?
            .ok_or_else(|| {
                format!(
                    "Gen4 device, {}, requires a relay channel, controller mapping, and GPIO configuration",
                    &assigned.device_id
                )
                .to_owned()
            })?;
        tracing::info!(
            "Dispatching Gen4 IPL request for device {} with target {:?}",
            &assigned.device_id,
            &target
        );
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
            test_execution::Entity::update_many()
                .col_expr(
                    test_execution::Column::ExecutionPhase,
                    crate::persistence::pg_enum::pg_enum(&test_execution::ExecutionPhase::SendFlashRequest),
                )
                .col_expr(test_execution::Column::UpdatedAt, Expr::value(chrono::Utc::now()))
                .filter(test_execution::Column::TestId.eq(test_id))
                .exec(database.orm())
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
                    test_execution::Entity::update_many()
                        .col_expr(
                            test_execution::Column::ExecutionPhase,
                            crate::persistence::pg_enum::pg_enum(&test_execution::ExecutionPhase::WaitForDeviceBoot),
                        )
                        .col_expr(
                            test_execution::Column::UpdatedAt,
                            Expr::value(chrono::Utc::now()),
                        )
                        .filter(test_execution::Column::TestId.eq(test_id))
                        .exec(database.orm())
                        .await
                        .map_err(|error| error.to_string())?;
                    let message = format!(
                        "Test execution request sent to device {} ({})",
                        assigned.device_id, assigned.ip_address
                    );
                    if let Err(error) =
                        crate::devices::test_log::append_test_log(state, test_id, &message).await
                    {
                        tracing::warn!(%error, %test_id, "failed to append test-execution dispatch log");
                    }
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

/// Return whether the device family string indicates Gen4 (needs IPL GPIO path).
async fn is_gen4_device(state: &AppState, device_id: &str) -> Result<bool, String> {
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable for Gen4 IPL".to_owned())?;
    let family = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .select_only()
        .column(device::Column::DeviceFamily)
        .into_tuple::<Option<String>>()
        .one(database.orm())
        .await
        .map_err(|error| error.to_string())?;
    Ok(family
        .flatten()
        .map(|value| value.to_lowercase().replace(' ', "") == "gen4")
        .unwrap_or(false))
}

/// Load relay serial, channel, and GPIO mapping for a Gen4 IPL flash target.
async fn resolve_gen4_ipl_target(
    state: &AppState,
    device_id: &str,
) -> Result<Option<Gen4IplTarget>, String> {
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable for Gen4 IPL".to_owned())?;
    let orm = database.orm();
    let device = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .one(orm)
        .await
        .map_err(|error| error.to_string())?;
    let Some(device) = device else {
        return Ok(None);
    };
    let family = device
        .device_family
        .as_deref()
        .unwrap_or_default()
        .to_lowercase()
        .replace(' ', "");
    if family != "gen4" {
        return Ok(None);
    }
    let device_mac = device
        .mac_address
        .clone()
        .unwrap_or_else(|| device.device_id.clone());
    let normalized_mac = normalize_mac(&device_mac);
    let channels = relay_channel::Entity::find()
        .filter(relay_channel::Column::DeviceId.eq(device_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .order_by_desc(relay_channel::Column::UpdatedAt)
        .all(orm)
        .await
        .map_err(|error| error.to_string())?;
    for channel in channels {
        let Some(relay_row) = relay::Entity::find_by_id(channel.relay_id)
            .filter(relay::Column::DeletedAt.is_null())
            .one(orm)
            .await
            .map_err(|error| error.to_string())?
        else {
            continue;
        };
        let Some(controller) =
            device_controller::Entity::find_by_id(relay_row.device_controller_id)
                .filter(device_controller::Column::DeletedAt.is_null())
                .one(orm)
                .await
                .map_err(|error| error.to_string())?
        else {
            continue;
        };
        let Some(mapping) = mapping_entry_for_mac(&controller.mappings, &normalized_mac) else {
            continue;
        };
        let gpio = mapping
            .get("gpio")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
        if gpio.is_empty() {
            continue;
        }
        return Ok(Some(Gen4IplTarget {
            controller_ip: controller.ip_address,
            relay_serial: relay_row.serial_number,
            channel_number: channel.channel_number,
            device_mac,
            gpio,
            gpio_default_level: mapping
                .get("gpioDefaultLevel")
                .and_then(|value| value.as_str())
                .unwrap_or("LOW")
                .to_owned(),
            relay_default_level: mapping
                .get("relayDefaultLevel")
                .and_then(|value| value.as_str())
                .unwrap_or("LOW")
                .to_owned(),
        }));
    }
    Ok(None)
}

/// Uppercase and colon-normalize a MAC for mapping table lookups.
fn normalize_mac(value: &str) -> String {
    value.to_ascii_lowercase().replace([':', '-'], "")
}

/// Find a controller mapping JSON entry whose MAC matches `mac`.
fn mapping_entry_for_mac<'a>(
    mappings: &'a serde_json::Value,
    normalized_mac: &str,
) -> Option<&'a serde_json::Value> {
    let object = mappings.as_object()?;
    object.iter().find_map(|(key, value)| {
        (normalize_mac(key) == normalized_mac).then_some(value)
    })
}

/// Drive Gen4 IPL mode via relay/GPIO then POST the flash payload to the DC.
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
    let transaction = database
        .orm()
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    let now = chrono::Utc::now();
    device_action_queue::Entity::update_many()
        .col_expr(
            device_action_queue::Column::Status,
            crate::persistence::pg_enum::pg_enum(&device_action_queue::QueueJobStatus::Waiting),
        )
        .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
        .filter(device_action_queue::Column::Id.eq(assigned.action.id))
        .exec(&transaction)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(test_id) = assigned.action.test_id.as_deref() {
        test_execution::Entity::update_many()
            .col_expr(
                test_execution::Column::ExecutionPhase,
                crate::persistence::pg_enum::pg_enum(&test_execution::ExecutionPhase::WaitForIplConfirmation),
            )
            .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
            .filter(test_execution::Column::TestId.eq(test_id))
            .exec(&transaction)
            .await
            .map_err(|error| error.to_string())?;
    }
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())?;
    if let Some(test_id) = assigned.action.test_id.as_deref() {
        let message = format!(
            "IPL flashing requested for device {} via controller {}",
            assigned.device_id, target.controller_ip
        );
        if let Err(error) = crate::devices::test_log::append_test_log(state, test_id, &message).await
        {
            tracing::warn!(%error, %test_id, "failed to append Gen4 IPL request log");
        }
    }
    Ok(())
}

/// Mark the action/test failed after dispatch error and build failure WS events.
async fn fail_dispatch(
    state: &AppState,
    assigned: &AssignedAction,
    error: &str,
) -> Result<Vec<ServerEvent>, sea_orm::DbErr> {
    let database = state.database.as_ref().expect("worker requires database");
    let transaction = database.orm().begin().await?;
    let now = chrono::Utc::now();

    device_action_queue::Entity::update_many()
        .col_expr(
            device_action_queue::Column::Status,
            crate::persistence::pg_enum::pg_enum(&device_action_queue::QueueJobStatus::Cancelled),
        )
        .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
        .filter(device_action_queue::Column::Id.eq(assigned.action.id))
        .exec(&transaction)
        .await?;

    device::Entity::update_many()
        .col_expr(
            device::Column::State,
            crate::persistence::pg_enum::pg_enum(&device::DeviceState::Free),
        )
        .col_expr(device::Column::StateUpdatedAt, Expr::value(Some(now)))
        .col_expr(device::Column::UpdatedAt, Expr::value(now))
        .filter(device::Column::DeviceId.eq(&assigned.device_id))
        .exec(&transaction)
        .await?;

    device_state_change::ActiveModel {
        device_id: Set(assigned.device_id.clone()),
        state: Set(device_state_change::DeviceStateChangeState::Free),
        changed_at: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&transaction)
    .await?;

    if let Some(test_id) = assigned.action.test_id.as_deref() {
        test_execution::Entity::update_many()
            .col_expr(
                test_execution::Column::Status,
                crate::persistence::pg_enum::pg_enum(&test_execution::TestStatus::Failed),
            )
            .col_expr(
                test_execution::Column::ExecutionPhase,
                crate::persistence::pg_enum::pg_enum(&test_execution::ExecutionPhase::CompleteFailed),
            )
            .col_expr(test_execution::Column::EndedAt, Expr::value(Some(now)))
            .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
            .filter(test_execution::Column::TestId.eq(test_id))
            .exec(&transaction)
            .await?;
    } else if let Some(release_id) = assigned.action.release_id {
        release::Entity::update_many()
            .col_expr(
                release::Column::Status,
                crate::persistence::pg_enum::pg_enum(&release::ReleaseTestStatus::Failed),
            )
            .col_expr(release::Column::IsFaulty, Expr::value(Some(true)))
            .col_expr(release::Column::UpdatedAt, Expr::value(now))
            .filter(release::Column::Id.eq(release_id))
            .exec(&transaction)
            .await?;
    }

    transaction.commit().await?;
    Ok(dispatch_failure_events(assigned, error))
}

/// Record a worker failure metric and log the sqlx-shaped error.
fn record_failure(state: &AppState, error: &sqlx::Error, message: &str) {
    state.metrics.record_worker_run("device_action", "failure");
    tracing::error!(%error, worker = "device_action", "{message}");
}

#[cfg(test)]
mod tests {
    use crate::{
        persistence::entity::device_action_queue::QueueDispatchMode,
        socket::helpers::assignment_events, workers::types::DeviceState,
    };
    use uuid::Uuid;

    use super::*;

    /// Test fixture: an AssignedAction for a TEST job.
    fn assigned_test() -> AssignedAction {
        AssignedAction {
            action: QueuedAction {
                id: Uuid::nil(),
                test_id: Some("test-1".to_owned()),
                release_id: Some(Uuid::nil()),
                version: None,
                device_type: "x4h".to_owned(),
                user_id: "user-1".to_owned(),
                mode: QueueDispatchMode::DeviceSpecific,
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
    /// Unit test: assignment event set matches frontend listeners.
    fn assignment_events_refresh_device_owner_and_build_views() {
        let events = assignment_events(&assigned_test());
        assert_eq!(
            events
                .iter()
                .map(|event| event.event.as_str())
                .collect::<Vec<_>>(),
            vec![
                "deviceStateUpdate",
                "testExecution",
                "testExecutionUpdate",
                "buildPerformanceUpdate",
            ]
        );
        assert_eq!(events[1].payload["data"]["status"], "inProgress");
    }

    #[test]
    /// Unit test: failure events free the device and fail the test.
    fn dispatch_failure_events_restore_device_and_fail_test() {
        let events = dispatch_failure_events(&assigned_test(), "connection refused");
        assert_eq!(events[0].payload["state"], DeviceState::Free.to_string());
        assert_eq!(events[1].payload["data"]["status"], "failed");
        assert_eq!(events[1].payload["data"]["message"], "connection refused");
    }
}
