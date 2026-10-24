//! Device action queue dispatcher (legacy `DeviceActionQueueService`).
//!
//! Polls `device_action_queue` for eligible queued jobs, claims one under
//! `FOR UPDATE SKIP LOCKED`, assigns a free device, and dispatches flash/test/IPL
//! work to the edge controller. Publishes assignment and failure farm events.

use std::{path::PathBuf, sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    devices::{DEVICE_ACTION_HTTP_TIMEOUT, DEVICE_COMMAND_RETRY_DELAY},
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
            // Wake immediately after prep enqueue / create (parity with Node setImmediate).
            _ = state.worker_wake.notified() => run_cycle(&state).await,
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
                tracing::trace!(worker = "device_action", "no queued actions available");
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
                    "Device {} is ready after IPL. Preparing and starting the selected tests...",
                    assigned.device_id
                )
            } else {
                format!(
                    "Starting test execution on device {}",
                    assigned.device_id
                )
            };
            if let Err(error) =
                crate::devices::test_log::append_test_log(state, test_id, &message).await
            {
                tracing::warn!(%error, %test_id, "failed to append assignment test log");
            }
        }
        tracing::debug!(action_id = %assigned.action.id, worker = "device_action", "dispatching action");
        match dispatch(state, &assigned).await {
            Ok(()) => {
                state.metrics.record_worker_items("device_action", 1);
                state.metrics.record_worker_run("device_action", "success");
            }
            Err(error) => {
                tracing::error!(%error, action_id = %assigned.action.id, worker = "device_action", "device dispatch failed");
                if let Some(test_id) = assigned.action.test_id.as_deref() {
                    let message =
                        crate::socket::helpers::user_friendly_dispatch_error(&error);
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
        // Queue columns keep Sequelize camelCase names; QueuedAction maps them via
        // #[sea_orm(alias = "...")]. Joined execution fields use snake_case aliases.
        .column(device_action_queue::Column::Id)
        .column(device_action_queue::Column::TestId)
        .column(device_action_queue::Column::ReleaseId)
        .column(device_action_queue::Column::Version)
        .column(device_action_queue::Column::DeviceType)
        .column(device_action_queue::Column::UserId)
        .column(device_action_queue::Column::Mode)
        .column(device_action_queue::Column::TargetDeviceId)
        .column(device_action_queue::Column::IplFlashConfirm)
        .column(device_action_queue::Column::CreatedAt)
        .column(device_action_queue::Column::UpdatedAt)
        .column_as(test_execution::Column::BuildId, "build_id")
        .column_as(test_execution::Column::BuildVersion, "build_version")
        .column_as(test_execution::Column::CreatedBy, "created_by")
        // Claim filter: only QUEUED rows. Gen5/x5h and official-build gates are applied
        // after assignment (Node DeviceActionQueueService parity) so jobs are never
        // silently stuck in QUEUED without logs.
        .filter(device_action_queue::Column::Status.eq(device_action_queue::QueueJobStatus::Queued))
        .filter(
            sea_orm::Condition::any()
                .add(device_action_queue::Column::Type.ne(device_action_queue::QueueJobType::Test))
                .add(
                    sea_orm::Condition::all()
                        // First assignment: fresh job still waiting in queue (NotExecuted or Queued).
                        // Post-IPL requeue: status is already in_progress and phase is
                        // REQUEUE_AFTER_IPL_CONFIRMATION.
                        .add(
                            sea_orm::Condition::any()
                                .add(
                                    sea_orm::Condition::all()
                                        .add(
                                            test_execution::Column::Status.is_in([
                                                test_execution::TestStatus::NotExecuted,
                                                test_execution::TestStatus::Queued,
                                            ]),
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

    tracing::trace!(worker = "device_action", "claimed next queue action");

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
        // No free device this tick. Distinguish "none compatible ever" (fail) from
        // "compatible but busy" (wait). Do NOT bump updated_at every poll — that reset
        // the 30s wait-log throttle so UI stayed silent after "added to queue".
        return handle_no_free_device(state, transaction, &action, confirmed).await;
    };

    // Official / faulty release gates (Node DeviceActionQueueService post-assign checks).
    if let Some(test_id) = action.test_id.as_deref()
        && let Some(outcome) =
            evaluate_release_gate(&transaction, &action, test_id).await?
    {
        apply_release_gate(&transaction, &action, test_id, &device_id, &outcome).await?;
        transaction.commit().await?;
        if let Err(error) =
            crate::devices::test_log::append_test_log(state, test_id, &outcome.message).await
        {
            tracing::warn!(%error, %test_id, "failed to append release-gate log");
        }
        return Ok(None);
    }

    // ---------------------------------------------------------------
    // 4. Mark queue action as running.
    // ---------------------------------------------------------------

    let action_model = device_action_queue::ActiveModel {
        id: Set(action.id),
        status: Set(Some(device_action_queue::QueueJobStatus::Running)),
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
        state: Set(Some(device_state_change::DeviceStateChangeState::Busy)),
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
/// Gen5 IPL confirmation request, and queue/test status updates on success paths.
async fn dispatch(state: &AppState, assigned: &AssignedAction) -> Result<(), String> {
    if assigned.action.ipl_flash_confirm.is_none() {
        if is_gen4_device(state, &assigned.device_id).await? {
            if device_already_on_target_version(state, assigned).await? {
                mark_ipl_confirm_true(state, assigned.action.id).await?;
            } else {
                let target = resolve_gen4_ipl_target(state, &assigned.device_id)
                    .await?
                    .ok_or_else(|| {
                        format!(
                            "Gen4 device, {}, requires a relay channel, controller mapping, and GPIO configuration",
                            &assigned.device_id
                        )
                    })?;
                tracing::debug!(
                    device_id = %assigned.device_id,
                    "dispatching Gen4 IPL request"
                );
                if let Some(test_id) = assigned.action.test_id.as_deref() {
                    let _ = crate::devices::test_log::append_test_log(
                        state,
                        test_id,
                        "Starting IPL flashing on the device. Progress updates will appear below...",
                    )
                    .await;
                }
                request_gen4_ipl(state, assigned, target).await?;
                return Ok(());
            }
        } else if is_gen5_device_type(&assigned.action.device_type) {
            if device_already_on_target_version(state, assigned).await? {
                mark_ipl_confirm_true(state, assigned.action.id).await?;
            } else {
                tracing::debug!(
                    device_id = %assigned.device_id,
                    "dispatching Gen5 IPL confirmation request"
                );
                if let Some(test_id) = assigned.action.test_id.as_deref() {
                    let _ = crate::devices::test_log::append_test_log(
                        state,
                        test_id,
                        "Starting IPL flashing on the device. Progress updates will appear below...",
                    )
                    .await;
                }
                request_gen5_ipl(state, assigned).await?;
                return Ok(());
            }
        }
    }

    let (path, payload) = if let Some(test_id) = assigned.action.test_id.as_deref() {
        // Node: buildVersion || buildId — empty build_ver is rejected by eagent (400).
        let build_version = dispatch_build_version(&assigned.action).ok_or_else(|| {
            "build version is missing for test-execution dispatch".to_owned()
        })?;
        let boot = resolve_test_boot_paths(state, assigned, test_id).await?;
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
        // Leading `/` on image/dtb satisfies deployed eagents that still require
        // absolute TFTP paths; U-Boot tftp treats them relative to the TFTP root.
        (
            "test-execution",
            serde_json::json!({
                "nfs": boot.nfs,
                "server_ip": state.config.device.server_ip,
                "image": boot.image,
                "dtb": boot.dtb,
                "testId": test_id,
                "build_ver": build_version,
                "deviceId": assigned.device_id,
            }),
        )
    } else {
        let version = assigned
            .action
            .version
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "flash version is missing for flash dispatch".to_owned())?;
        let build_path = format!("{}/{version}", assigned.action.device_type);
        (
            "flash",
            serde_json::json!({
                "nfs": format!("{}/{build_path}", state.config.device.nfs_host_path.trim_end_matches('/')),
                "server_ip": state.config.device.server_ip,
                "image": format!("/{build_path}/Image"),
                "dtb": format!("/{build_path}/board.dtb"),
                "build_ver": version,
                "deviceId": assigned.device_id,
            }),
        )
    };
    let agent_port = state.config.device.edge_agent_http_port;
    let url = reqwest::Url::parse(&format!(
        "http://{}:{agent_port}/{path}",
        assigned.ip_address
    ))
    .map_err(|error| error.to_string())?;
    let client = crate::external_http::client(Duration::from_secs(
        state.config.device.request_timeout_seconds,
    ))
    .map_err(|error| error.to_string())?;
    let mut last_error = String::new();
    for _ in 0..crate::external_http::EXPLICIT_COMMAND_ATTEMPTS {
        let request = crate::external_http::with_edge_agent_auth(client.post(url.clone()).json(&payload));
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                let body = response.text().await.unwrap_or_default();
                let started = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|value| value.get("started")?.as_bool())
                    .unwrap_or(false);
                let rebooting = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|value| value.get("rebooting")?.as_bool())
                    .unwrap_or(!started);
                if let Some(test_id) = assigned.action.test_id.as_deref()
                    && let Some(database) = &state.database
                {
                    // Same-root eagent starts immediately (`started`); reboot path waits for boot.
                    let phase = if started {
                        test_execution::ExecutionPhase::StartTestExecution
                    } else {
                        test_execution::ExecutionPhase::WaitForDeviceBoot
                    };
                    test_execution::Entity::update_many()
                        .col_expr(
                            test_execution::Column::ExecutionPhase,
                            crate::persistence::pg_enum::pg_enum(&phase),
                        )
                        .col_expr(
                            test_execution::Column::UpdatedAt,
                            Expr::value(chrono::Utc::now()),
                        )
                        .filter(test_execution::Column::TestId.eq(test_id))
                        .exec(database.orm())
                        .await
                        .map_err(|error| error.to_string())?;
                    let message = if started {
                        format!(
                            "Selected tests started on device {} ({}). Waiting for case results...",
                            assigned.device_id, assigned.ip_address
                        )
                    } else {
                        format!(
                            "Selected tests were sent to device {} ({}). Waiting for the device to boot and report case results...",
                            assigned.device_id, assigned.ip_address
                        )
                    };
                    if let Err(error) =
                        crate::devices::test_log::append_test_log(state, test_id, &message).await
                    {
                        tracing::warn!(%error, %test_id, "failed to append test-execution dispatch log");
                    }
                    if started {
                        crate::socket::helpers::publish_result_progress(
                            state,
                            test_id,
                            assigned.action.build_id.as_deref(),
                            0,
                            crate::socket::types::frontend::TestStatus::InProgress,
                            Some("Selected tests are running on the device"),
                        );
                    } else {
                        let _ = rebooting;
                        crate::socket::helpers::publish_device_boot_progress(
                            state,
                            test_id,
                            assigned.action.build_id.as_deref(),
                            &message,
                        );
                    }
                }
                return Ok(());
            }
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let body = body.trim();
                last_error = if body.is_empty() {
                    format!("device returned {status}")
                } else {
                    format!("device returned {status}: {body}")
                };
            }
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(DEVICE_COMMAND_RETRY_DELAY).await;
    }
    Err(last_error)
}

/// Resolve build_ver for device flash/test payloads (Node: buildVersion || buildId).
fn dispatch_build_version(action: &QueuedAction) -> Option<&str> {
    action
        .build_version
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            action
                .version
                .as_deref()
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            action
                .build_id
                .as_deref()
                .filter(|value| !value.is_empty())
        })
}

#[derive(Debug)]
struct ReleaseGateOutcome {
    message: String,
    /// When true, cancel the queue job and fail/cancel the execution; otherwise leave queued.
    cancel: bool,
}

/// PURPOSE: Apply Node-parity official/faulty release gates after a device is chosen.
/// USE: Before marking the action running so silent SQL excludes are replaced with logs.
async fn evaluate_release_gate(
    transaction: &impl sea_orm::ConnectionTrait,
    action: &QueuedAction,
    test_id: &str,
) -> Result<Option<ReleaseGateOutcome>, sea_orm::DbErr> {
    let Some(release_id) = action.release_id.or_else(|| {
        action
            .build_id
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok())
    }) else {
        return Ok(None);
    };
    let Some(release) = release::Entity::find_by_id(release_id)
        .one(transaction)
        .await?
    else {
        return Ok(None);
    };
    if release.is_faulty {
        return Ok(Some(ReleaseGateOutcome {
            message: "Test cancelled: Release is marked as faulty".to_owned(),
            cancel: true,
        }));
    }
    if release.build_type == release::BuildType::Official
        && release.status != Some(release::ReleaseTestStatus::Passed)
    {
        let status = match release.status.as_ref() {
            Some(release::ReleaseTestStatus::Untested) => "untested",
            Some(release::ReleaseTestStatus::Testing) => "testing",
            Some(release::ReleaseTestStatus::Failed) => "failed",
            Some(release::ReleaseTestStatus::Passed) => "passed",
            None => "unknown",
        };
        if matches!(status, "untested" | "testing") {
            return Ok(Some(ReleaseGateOutcome {
                message: format!(
                    "Test re-queued: Official build status is '{status}', waiting for validation"
                ),
                cancel: false,
            }));
        }
        return Ok(Some(ReleaseGateOutcome {
            message: format!(
                "Test cancelled: Official build has failed status '{status}'"
            ),
            cancel: true,
        }));
    }
    let _ = test_id;
    Ok(None)
}

/// Handle assign tick when no free eligible device was claimed.
///
/// Parity with Node `ensureQueuedTestsHaveEligibleDevices`: fail the test when no
/// approved device can ever satisfy preconditions; otherwise keep queued and
/// throttle UI wait logs to ~30s using `updated_at` as the last-log watermark.
async fn handle_no_free_device(
    state: &AppState,
    transaction: sea_orm::DatabaseTransaction,
    action: &QueuedAction,
    confirmed: bool,
) -> Result<Option<AssignedAction>, sea_orm::DbErr> {
    let approved = device::Entity::find()
        .filter(device::Column::DeviceType.eq(&action.device_type))
        .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
        .filter(device::Column::DeletedAt.is_null())
        .filter(device::Column::IpAddress.is_not_null())
        .select_only()
        .column(device::Column::DeviceId)
        .column(device::Column::State)
        .into_tuple::<(String, Option<device::DeviceState>)>()
        .all(&transaction)
        .await?;

    let approved_ids: Vec<String> = if matches!(
        action.mode,
        device_action_queue::QueueDispatchMode::DeviceSpecific
    ) || confirmed
    {
        let Some(target) = action.target_device_id.as_deref() else {
            transaction.rollback().await?;
            if let Some(test_id) = action.test_id.as_deref() {
                fail_unmet_conditions(
                    state,
                    action,
                    test_id,
                    "Conditions not met - target device is missing",
                )
                .await?;
            }
            return Ok(None);
        };
        approved
            .into_iter()
            .filter(|(id, _)| id == target)
            .map(|(id, _)| id)
            .collect()
    } else {
        approved.into_iter().map(|(id, _)| id).collect()
    };

    if approved_ids.is_empty() {
        transaction.rollback().await?;
        if let Some(test_id) = action.test_id.as_deref() {
            let message = if matches!(
                action.mode,
                device_action_queue::QueueDispatchMode::DeviceSpecific
            ) || confirmed
            {
                format!(
                    "Conditions not met - target device {} is not approved/available",
                    action.target_device_id.as_deref().unwrap_or("unknown")
                )
            } else {
                format!(
                    "Conditions not met - no approved {} devices available",
                    action.device_type
                )
            };
            fail_unmet_conditions(state, action, test_id, &message).await?;
        }
        return Ok(None);
    }

    // Any approved device that satisfies preconditions (free or busy) keeps the job queued.
    let mut has_compatible = confirmed || action.test_id.is_none();
    if !has_compatible
        && let Some(test_id) = action.test_id.as_deref()
    {
        for device_id in &approved_ids {
            if test_preconditions::device_satisfies(&transaction, device_id, test_id).await? {
                has_compatible = true;
                break;
            }
        }
        if !has_compatible {
            transaction.rollback().await?;
            fail_unmet_conditions(
                state,
                action,
                test_id,
                "Conditions not met - no approved device satisfies test preconditions",
            )
            .await?;
            return Ok(None);
        }
    }

    // Compatible device(s) exist but none are free right now — wait, throttle UI logs.
    // Log immediately once (updated_at still ≈ created_at), then at most every ~30s.
    let now = chrono::Utc::now();
    let age_secs = now.signed_duration_since(action.updated_at).num_seconds();
    let first_wait = action
        .updated_at
        .signed_duration_since(action.created_at)
        .num_seconds()
        .abs()
        < 2;
    let should_log_wait = first_wait || age_secs >= 30;
    if should_log_wait {
        device_action_queue::Entity::update_many()
            .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
            .filter(device_action_queue::Column::Id.eq(action.id))
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        if let Some(test_id) = action.test_id.as_deref() {
            let message = format!(
                "Waiting for a free {} device that satisfies test preconditions ({} approved)",
                action.device_type,
                approved_ids.len()
            );
            if let Err(error) =
                crate::devices::test_log::append_test_log(state, test_id, &message).await
            {
                tracing::warn!(%error, %test_id, "failed to append device-wait log");
            }
        }
        tracing::debug!(
            action_id = %action.id,
            device_type = %action.device_type,
            approved = approved_ids.len(),
            worker = "device_action",
            "queued action waiting for a free device"
        );
    } else {
        transaction.rollback().await?;
    }
    Ok(None)
}

/// Mark queue + test failed when no device can satisfy assignment conditions.
async fn fail_unmet_conditions(
    state: &AppState,
    action: &QueuedAction,
    test_id: &str,
    reason: &str,
) -> Result<(), sea_orm::DbErr> {
    let database = state.database.as_ref().expect("worker requires database");
    let transaction = database.orm().begin().await?;
    let now = chrono::Utc::now();

    let execution = test_execution::Entity::find()
        .filter(test_execution::Column::TestId.eq(test_id))
        .one(&transaction)
        .await?;
    if let Some(execution) = execution.as_ref()
        && matches!(
            execution.status,
            Some(
                test_execution::TestStatus::Completed
                    | test_execution::TestStatus::Failed
                    | test_execution::TestStatus::Cancelled
            )
        )
    {
        device_action_queue::Entity::update_many()
            .col_expr(
                device_action_queue::Column::Status,
                crate::persistence::pg_enum::pg_enum(
                    &device_action_queue::QueueJobStatus::Completed,
                ),
            )
            .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
            .filter(device_action_queue::Column::Id.eq(action.id))
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        return Ok(());
    }

    device_action_queue::Entity::update_many()
        .col_expr(
            device_action_queue::Column::Status,
            crate::persistence::pg_enum::pg_enum(&device_action_queue::QueueJobStatus::Cancelled),
        )
        .col_expr(
            device_action_queue::Column::TargetDeviceId,
            Expr::value(Option::<String>::None),
        )
        .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
        .filter(device_action_queue::Column::Id.eq(action.id))
        .exec(&transaction)
        .await?;
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
    transaction.commit().await?;

    if let Err(error) = crate::devices::test_log::append_test_log(state, test_id, reason).await {
        tracing::warn!(%error, %test_id, "failed to append unmet-conditions log");
    }
    tracing::warn!(
        action_id = %action.id,
        %test_id,
        worker = "device_action",
        "{reason}"
    );
    Ok(())
}

async fn apply_release_gate(
    transaction: &impl sea_orm::ConnectionTrait,
    action: &QueuedAction,
    test_id: &str,
    _device_id: &str,
    outcome: &ReleaseGateOutcome,
) -> Result<(), sea_orm::DbErr> {
    let now = chrono::Utc::now();
    if outcome.cancel {
        device_action_queue::Entity::update_many()
            .col_expr(
                device_action_queue::Column::Status,
                crate::persistence::pg_enum::pg_enum(
                    &device_action_queue::QueueJobStatus::Cancelled,
                ),
            )
            .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
            .filter(device_action_queue::Column::Id.eq(action.id))
            .exec(transaction)
            .await?;
        test_execution::Entity::update_many()
            .col_expr(
                test_execution::Column::Status,
                crate::persistence::pg_enum::pg_enum(&test_execution::TestStatus::Cancelled),
            )
            .col_expr(
                test_execution::Column::ExecutionPhase,
                crate::persistence::pg_enum::pg_enum(
                    &test_execution::ExecutionPhase::CompleteCancelled,
                ),
            )
            .col_expr(test_execution::Column::EndedAt, Expr::value(Some(now)))
            .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
            .filter(test_execution::Column::TestId.eq(test_id))
            .exec(transaction)
            .await?;
    } else {
        // Leave queued; bump updatedAt so watchdog/ordering stay honest while waiting.
        device_action_queue::Entity::update_many()
            .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
            .filter(device_action_queue::Column::Id.eq(action.id))
            .exec(transaction)
            .await?;
    }
    Ok(())
}

fn is_gen5_device_type(device_type: &str) -> bool {
    device_type.to_ascii_lowercase().contains("x5h")
}

async fn device_already_on_target_version(
    state: &AppState,
    assigned: &AssignedAction,
) -> Result<bool, String> {
    Ok(load_assigned_device(state, assigned)
        .await?
        .is_some_and(|device| device_software_matches_target(&device, assigned)))
}

/// PURPOSE: Choose Mode A (reuse registered NFS/TFTP) vs Mode B (new `{type}/{testId}` tree).
/// USE: test-execution payload construction after optional IPL confirmation.
///
/// Mode A when the DUT already reports the target `build_ver` and has a usable `nfsPath`
/// (IPL may already have been skipped). Otherwise Mode B stages/boots the full image tree
/// prepared from the release ZIP under `{nfs_host}/{deviceType}/{testId}`.
async fn resolve_test_boot_paths(
    state: &AppState,
    assigned: &AssignedAction,
    test_id: &str,
) -> Result<super::nfs_boot_paths::BootPaths, String> {
    let staging = super::nfs_boot_paths::staging_nfs_root(
        &state.config.device.nfs_host_path,
        &assigned.action.device_type,
        test_id,
    );
    let device = load_assigned_device(state, assigned).await?;
    let reuse = device
        .as_ref()
        .is_some_and(|row| device_software_matches_target(row, assigned));

    if reuse {
        let nfs_path = device
            .as_ref()
            .and_then(|row| row.nfs_path.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!(
                    "device {} is already on the target build but has no registered nfsPath; re-register the device before running tests without a full image flash",
                    assigned.device_id
                )
            })?;
        let boot = super::nfs_boot_paths::derive_boot_paths(
            &state.config.device.nfs_host_path,
            nfs_path,
        )?;
        let current_root = PathBuf::from(&boot.nfs);
        let staging_root = staging.clone();
        tokio::task::spawn_blocking(move || {
            super::nfs_boot_paths::sync_usr_src_tree(&staging_root, &current_root)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
        let current_test = PathBuf::from(&boot.nfs).join("usr/src/currentTest.txt");
        if let Some(parent) = current_test.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| error.to_string())?;
        }
        tokio::fs::write(&current_test, test_id)
            .await
            .map_err(|error| error.to_string())?;
        tracing::info!(
            device_id = %assigned.device_id,
            test_id,
            nfs = %boot.nfs,
            "reusing registered NFS/TFTP paths for test execution (Mode A)"
        );
        return Ok(boot);
    }

    let build_path = format!("{}/{test_id}", assigned.action.device_type);
    let current_test = staging.join("usr/src/currentTest.txt");
    if let Some(parent) = current_test.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
    }
    tokio::fs::write(&current_test, test_id)
        .await
        .map_err(|error| error.to_string())?;
    tracing::info!(
        device_id = %assigned.device_id,
        test_id,
        build_path = %build_path,
        "using new per-test NFS/TFTP paths for full image flash (Mode B)"
    );
    Ok(super::nfs_boot_paths::BootPaths {
        nfs: format!(
            "{}/{build_path}",
            state.config.device.nfs_host_path.trim_end_matches('/')
        ),
        image: format!("/{build_path}/Image"),
        dtb: format!("/{build_path}/board.dtb"),
    })
}

async fn load_assigned_device(
    state: &AppState,
    assigned: &AssignedAction,
) -> Result<Option<device::Model>, String> {
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable".to_owned())?;
    device::Entity::find()
        .filter(device::Column::DeviceId.eq(&assigned.device_id))
        .filter(device::Column::DeletedAt.is_null())
        .one(database.orm())
        .await
        .map_err(|error| error.to_string())
}

fn device_software_matches_target(device: &device::Model, assigned: &AssignedAction) -> bool {
    let target = assigned
        .action
        .build_version
        .as_deref()
        .or(assigned.action.version.as_deref())
        .unwrap_or("")
        .trim();
    if target.is_empty() {
        return false;
    }
    device
        .software_version
        .as_deref()
        .map(|value| value.trim() == target)
        .unwrap_or(false)
}

async fn mark_ipl_confirm_true(state: &AppState, action_id: Uuid) -> Result<(), String> {
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable".to_owned())?;
    let now = chrono::Utc::now();
    device_action_queue::Entity::update_many()
        .col_expr(device_action_queue::Column::IplFlashConfirm, Expr::value(Some(true)))
        .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
        .filter(device_action_queue::Column::Id.eq(action_id))
        .exec(database.orm())
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// PURPOSE: Request Gen5 IPL flash confirmation from the EdgeController (Node askFlashConfirmation).
/// USE: First dispatch of x5h jobs when the DUT is not already on the target version.
async fn request_gen5_ipl(state: &AppState, assigned: &AssignedAction) -> Result<(), String> {
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable for Gen5 IPL".to_owned())?;
    let orm = database.orm();
    let device = device::Entity::find()
        .filter(device::Column::DeviceId.eq(&assigned.device_id))
        .filter(device::Column::DeletedAt.is_null())
        .one(orm)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("device {} not found for Gen5 IPL", assigned.device_id))?;
    let device_mac = device
        .mac_address
        .clone()
        .unwrap_or_else(|| device.device_id.clone());
    let normalized_mac = normalize_mac(&device_mac);

    let mut controller = if let Some(controller_id) = device.controller_id.as_deref() {
        device_controller::Entity::find_by_id(controller_id.to_owned())
            .filter(device_controller::Column::DeletedAt.is_null())
            .one(orm)
            .await
            .map_err(|error| error.to_string())?
    } else {
        None
    };
    if controller
        .as_ref()
        .and_then(|row| mapping_entry_for_mac(&row.mappings, &normalized_mac))
        .is_none()
    {
        let controllers = device_controller::Entity::find()
            .filter(
                device_controller::Column::Status
                    .eq(device_controller::DeviceControllerStatus::Approved),
            )
            .filter(
                device_controller::Column::State.eq(device_controller::DeviceControllerState::Active),
            )
            .filter(device_controller::Column::DeletedAt.is_null())
            .all(orm)
            .await
            .map_err(|error| error.to_string())?;
        controller = controllers.into_iter().find(|row| {
            mapping_entry_for_mac(&row.mappings, &normalized_mac).is_some()
        });
    }
    let controller = controller.ok_or_else(|| {
        format!(
            "No controller UART/power mapping found for Gen5 device {}",
            assigned.device_id
        )
    })?;
    let mapping = mapping_entry_for_mac(&controller.mappings, &normalized_mac).ok_or_else(|| {
        format!(
            "No controller UART/power mapping found for Gen5 device {}",
            assigned.device_id
        )
    })?;
    let uart = mapping
        .get("uart")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let power = mapping
        .get("power")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if uart.is_empty() || power.is_empty() {
        return Err(format!(
            "Incomplete UART/power mapping for Gen5 device {}",
            assigned.device_id
        ));
    }
    let version = assigned
        .action
        .build_version
        .as_deref()
        .or(assigned.action.version.as_deref())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Gen5 IPL build version is missing".to_owned())?;
    let path = format!(
        "{}/{}",
        state
            .config
            .device
            .gen5_ipl_remote_path
            .trim_end_matches('/'),
        version
    );
    let payload = serde_json::json!({
        "gen": 5,
        "uart": uart,
        "power": power,
        "sdk_ver": version,
        "path": path,
        "controller_ip": controller.ip_address,
        "mac": device_mac,
    });
    let controller_port = state.config.device.edge_controller_http_port;
    let url = format!("http://{}:{controller_port}/ipl", controller.ip_address);
    let client = crate::external_http::client(DEVICE_ACTION_HTTP_TIMEOUT)
        .map_err(|error| error.to_string())?;
    // Fire-and-forget parity with Node askFlashConfirmation (timeouts still return Ok).
    let _ = client.post(&url).json(&payload).send().await;

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
        .col_expr(
            device_action_queue::Column::IplFlashRequestSent,
            Expr::value(Some(true)),
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
                crate::persistence::pg_enum::pg_enum(
                    &test_execution::ExecutionPhase::WaitForIplConfirmation,
                ),
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
        if let Err(error) = crate::devices::test_log::append_test_log(
            state,
            test_id,
            "Waiting for IPL flash confirmation",
        )
        .await
        {
            tracing::warn!(%error, %test_id, "failed to append Gen5 IPL wait log");
        }
    }
    Ok(())
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
    let controller_port = state.config.device.edge_controller_http_port;
    let base = format!("http://{}:{controller_port}", target.controller_ip);
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
        state: Set(Some(device_state_change::DeviceStateChangeState::Free)),
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
            .col_expr(release::Column::IsFaulty, Expr::value(true))
            .col_expr(release::Column::UpdatedAt, Expr::value(now))
            .filter(release::Column::Id.eq(release_id))
            .exec(&transaction)
            .await?;
    }

    transaction.commit().await?;
    if let Some(test_id) = assigned.action.test_id.as_deref() {
        let mut paths = vec![
            PathBuf::from(&state.config.device.nfs_host_path)
                .join(&assigned.action.device_type)
                .join(test_id)
                .join("usr/src/currentTest.txt"),
        ];
        if let Ok(Some(device)) = load_assigned_device(state, assigned).await
            && let Some(nfs_path) = device.nfs_path.as_deref().map(str::trim).filter(|v| !v.is_empty())
        {
            paths.push(PathBuf::from(nfs_path).join("usr/src/currentTest.txt"));
        }
        for path in paths {
            if let Err(error) = tokio::fs::remove_file(&path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, path = %path.display(), %test_id, "failed to clear NFS currentTest after dispatch failure");
            }
        }
    }
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
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
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
