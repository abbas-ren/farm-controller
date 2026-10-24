//! Device state transition rules during heartbeat processing.
//!
//! Encodes busy/free/faulty/not_reachable transitions and test-log inserts
//! matching Node heartbeat worker behavior.

use sea_orm::{ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect, Set,
    sea_query::{Expr, LockType},
};

use crate::{
    devices::repository_types::{DeferredCancellation, DeferredTestLog},
    persistence::entity::{
        device, device_action_queue, fallback_update, log_entry, release, test_execution,
    },
    socket::types::{farm::ServerEvent, frontend::FrontendMessagePayload},
};

/// PURPOSE: Apply heartbeat-confirmed recovery (fallback flash, action queue, cancel busy tests).
/// USE: Inside `record_device` after the device row is updated, still in the same transaction.
pub(super) async fn process_device_post_update(
    transaction: &impl ConnectionTrait,
    device_id: &str,
    nfs_path: Option<&str>,
) -> Result<(Vec<ServerEvent>, Vec<DeferredCancellation>, Vec<DeferredTestLog>), sea_orm::DbErr>
{
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::heartbeat_store::state_transition",
        "process_device_post_update",
    );
    let mut events = Vec::new();
    let mut deferred_cancellations = Vec::new();
    let mut deferred_test_logs = Vec::new();
    let now = chrono::Utc::now();

    let fallback = fallback_update::Entity::find()
        .filter(fallback_update::Column::DeviceId.eq(device_id))
        .filter(fallback_update::Column::Status.is_in([
            fallback_update::FallbackStatus::Pending,
            fallback_update::FallbackStatus::Flashing,
        ]))
        .order_by_desc(fallback_update::Column::CreatedAt)
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
        .one(transaction)
        .await?;
    if let Some(fallback) = fallback {
        let matched = nfs_path == Some(fallback.fallback_path.as_str());
        let phase = if matched {
            "COMPLETE_SUCCESS"
        } else {
            "COMPLETE_FAILED"
        };
        let mut fallback_active = fallback.clone().into_active_model();
        fallback_active.status = Set(if matched {
            fallback_update::FallbackStatus::Completed
        } else {
            fallback_update::FallbackStatus::Failed
        });
        fallback_active.updated_at = Set(now);
        fallback_active.update(transaction).await?;

        let execution_phase = if matched {
            test_execution::ExecutionPhase::CompleteSuccess
        } else {
            test_execution::ExecutionPhase::CompleteFailed
        };
        test_execution::Entity::update_many()
            .col_expr(
                test_execution::Column::ExecutionPhase,
                crate::persistence::pg_enum::pg_enum(&execution_phase),
            )
            .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
            .filter(test_execution::Column::TestId.eq(&fallback.test_id))
            .exec(transaction)
            .await?;
        events.push(ServerEvent::frontend(
            FrontendMessagePayload::BuildTestExecutionUpdate {
                test_id: fallback.test_id.clone(),
                build_id: "".to_owned(),
                test_case_id: None,
                status: Some(phase.to_owned()),
            },
        ));
    }

    let completed_flash = device_action_queue::Entity::find()
        .filter(device_action_queue::Column::TargetDeviceId.eq(device_id))
        .filter(device_action_queue::Column::Type.eq(device_action_queue::QueueJobType::Flash))
        .filter(device_action_queue::Column::Status.eq(device_action_queue::QueueJobStatus::Running))
        .order_by_asc(device_action_queue::Column::CreatedAt)
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
        .one(transaction)
        .await?;
    if let Some(action) = completed_flash {
        let mut active = action.into_active_model();
        active.status = Set(device_action_queue::QueueJobStatus::Completed);
        active.updated_at = Set(now);
        active.update(transaction).await?;

        if let Some(device_row) = device::Entity::find()
            .filter(device::Column::DeviceId.eq(device_id))
            .one(transaction)
            .await?
        {
            let releases = release::Entity::find()
                .filter(release::Column::TestedDeviceId.eq(device_id))
                .filter(release::Column::Status.eq(release::ReleaseTestStatus::Testing))
                .all(transaction)
                .await?;
            for release_row in releases {
                let mut release_active = release_row.clone().into_active_model();
                // Status transition: fallback flash success marks the release passed.
                release_active.status = Set(Some(release::ReleaseTestStatus::Passed));
                release_active.is_faulty = Set(false);
                release_active.updated_at = Set(now);
                release_active.update(transaction).await?;
                events.push(ServerEvent::frontend(
                    FrontendMessagePayload::BuildStatusChanged {
                        release_id: release_row.id,
                        status: "passed".to_owned(),
                        is_faulty: false,
                        device_id: Some(device_id.to_owned()),
                        device_type: device_row
                            .device_type
                            .clone()
                            .unwrap_or_default(),
                        version: Some(release_row.version.clone()),
                        user_id: release_row.uploaded_by.clone(),
                    },
                ));
            }
        }
    }

    let flash_confirm = device_action_queue::Entity::find()
        .filter(device_action_queue::Column::TargetDeviceId.eq(device_id))
        .filter(device_action_queue::Column::Status.eq(device_action_queue::QueueJobStatus::Waiting))
        .filter(device_action_queue::Column::IplFlashConfirm.eq(true))
        .order_by_asc(device_action_queue::Column::CreatedAt)
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
        .one(transaction)
        .await?;
    if let Some(action) = flash_confirm {
        let test_id = action.test_id.clone();
        let can_requeue = if let Some(tid) = test_id.as_deref() {
            let execution = test_execution::Entity::find()
                .filter(test_execution::Column::TestId.eq(tid))
                // Row lock: serialize concurrent updates on this controller/device/relay.
                .lock(LockType::Update)
                .one(transaction)
                .await?;
            execution
                .map(|e| !e.cancel_requested.unwrap_or(false))
                .unwrap_or(false)
        } else {
            true
        };

        if can_requeue {
            let mut active = action.into_active_model();
            active.status = Set(device_action_queue::QueueJobStatus::Queued);
            active.mode = Set(device_action_queue::QueueDispatchMode::DeviceSpecific);
            active.target_device_id = Set(Some(device_id.to_owned()));
            active.updated_at = Set(now);
            active.update(transaction).await?;
            if let Some(tid) = test_id.as_deref() {
                test_execution::Entity::update_many()
                    .col_expr(
                        test_execution::Column::ExecutionPhase,
                        crate::persistence::pg_enum::pg_enum(
                            &test_execution::ExecutionPhase::RequeueAfterIplConfirmation,
                        ),
                    )
                    .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
                    .filter(test_execution::Column::TestId.eq(tid))
                    .exec(transaction)
                    .await?;
            }
        } else if let Some(tid) = test_id.as_deref() {
            let execution = test_execution::Entity::find()
                .filter(test_execution::Column::TestId.eq(tid))
                .one(transaction)
                .await?;
            if let Some(execution) = execution {
                let terminal = matches!(
                    execution.status,
                    // Status transition: cancel in-progress execution when heartbeat frees a busy device.
                    Some(test_execution::TestStatus::Cancelled)
                        | Some(test_execution::TestStatus::Completed)
                        | Some(test_execution::TestStatus::Failed)
                );
                if !terminal {
                    let build_id = execution.build_id.clone();
                    let created_by = execution.created_by.clone();
                    let started = execution.started_at.unwrap_or(now);
                    let mut active = execution.into_active_model();
                    // Status transition: cancel in-progress execution when heartbeat frees a busy device.
                    active.status = Set(Some(test_execution::TestStatus::Cancelled));
                    active.cancel_handled = Set(Some(true));
                    active.execution_phase =
                        Set(Some(test_execution::ExecutionPhase::CompleteCancelled));
                    active.started_at = Set(Some(started));
                    active.ended_at = Set(Some(now));
                    active.updated_at = Set(now);
                    active.update(transaction).await?;

                    device_action_queue::Entity::update_many()
                        .col_expr(
                            device_action_queue::Column::Status,
                            crate::persistence::pg_enum::pg_enum(&device_action_queue::QueueJobStatus::Cancelled),
                        )
                        .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
                        .filter(device_action_queue::Column::TestId.eq(tid))
                        .filter(device_action_queue::Column::Status.is_not_in([
                            device_action_queue::QueueJobStatus::Completed,
                            device_action_queue::QueueJobStatus::Cancelled,
                        ]))
                        .exec(transaction)
                        .await?;

                    insert_test_log(
                        transaction,
                        tid,
                        "This test has been cancelled successfully",
                        now,
                    )
                    .await?;

                    deferred_cancellations.push(DeferredCancellation {
                        test_id: tid.to_owned(),
                        build_id,
                        created_by: created_by.unwrap_or_else(|| "system".to_owned()),
                    });
                }
            }
        }

        if let Some(tid) = test_id.as_deref() {
            let message = if can_requeue {
                format!(
                    "Device {device_id} is back online after IPL flashing, continuing the execution"
                )
            } else {
                format!(
                    "Device {device_id} is back online after IPL flashing; requeue skipped due to cancellation request"
                )
            };
            insert_test_log(transaction, tid, &message, now).await?;
            deferred_test_logs.push(DeferredTestLog {
                test_id: tid.to_owned(),
                message,
            });
        }
        return Ok((events, deferred_cancellations, deferred_test_logs));
    }

    let running_test = device_action_queue::Entity::find()
        .filter(device_action_queue::Column::TargetDeviceId.eq(device_id))
        .filter(device_action_queue::Column::Type.eq(device_action_queue::QueueJobType::Test))
        .filter(device_action_queue::Column::Status.eq(device_action_queue::QueueJobStatus::Running))
        .filter(device_action_queue::Column::TestId.is_not_null())
        .order_by_asc(device_action_queue::Column::CreatedAt)
        .one(transaction)
        .await?;
    if let Some(action) = running_test {
        if let Some(tid) = action.test_id.as_deref() {
            test_execution::Entity::update_many()
                .col_expr(
                    test_execution::Column::ExecutionPhase,
                    crate::persistence::pg_enum::pg_enum(&test_execution::ExecutionPhase::StartTestExecution),
                )
                .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
                .filter(test_execution::Column::TestId.eq(tid))
                .filter(
                    Condition::any()
                        .add(test_execution::Column::CancelRequested.eq(false))
                        .add(test_execution::Column::CancelRequested.is_null()),
                )
                .exec(transaction)
                .await?;
        }
    }
    Ok((events, deferred_cancellations, deferred_test_logs))
}

/// PURPOSE: Create/persist `insert test log`.
/// USE: Create handlers after validation.
async fn insert_test_log(
    transaction: &impl ConnectionTrait,
    test_id: &str,
    message: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), sea_orm::DbErr> {
    log_entry::ActiveModel {
        r#type: Set(log_entry::LogEntryType::Test),
        reference_id: Set(test_id.to_owned()),
        data: Set(serde_json::json!({ "message": message })),
        level: Set(log_entry::LogEntryLevel::Info),
        timestamp: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}
