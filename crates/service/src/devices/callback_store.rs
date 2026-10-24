//! Persist Gen5 mapping callbacks and flash-confirmation side effects.
//!
//! Invoked from device callback HTTP handlers after edge controllers report
//! mapping or IPL flash status. Ports Node `DeviceService` Gen5 mapping save
//! and flash confirmation that may cancel the active execution and free the device.

use chrono::{Duration, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, EntityTrait,
    FromQueryResult, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, LockBehavior, LockType},
};
use tracing::info;

use crate::{
    devices::{FlashExecutionProgress, IplFlashingStatus},
    persistence::entity::{
        device, device_action_queue, device_controller, device_state_change, gen5_mapping_queue,
        log_entry, test_execution,
    },
};

use super::{
    CallbackStatus, TtyEntry,
    error::DeviceRepositoryError,
    repository_types::{FlashCancelledExecution, FlashConfirmationResult},
    validation::normalize_mapping_mac,
};

/// PURPOSE: Persist a Gen5 mapping success/failure callback and claim matching queue jobs.
/// USE: Mapping callback HTTP handler after the edge reports UART mapping status.
pub(crate) async fn save_gen5_mapping(
    orm: &DatabaseConnection,
    caller_ip: &str,
    mac: &str,
    status: CallbackStatus,
    tty_entry: Option<&TtyEntry>,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::callback_store", "save_gen5_mapping");
    let transaction = orm.begin().await?;
    let controller = device_controller::Entity::find()
        .filter(device_controller::Column::IpAddress.eq(caller_ip))
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
        .one(&transaction)
        .await?;
    let Some(controller) = controller else {
        transaction.commit().await?;
        return Ok(());
    };

    match status {
        CallbackStatus::Success => {
            if let Some(tty_entry) = tty_entry {
                let normalized_mac = normalize_mapping_mac(mac)?;
                let mut mappings = controller.mappings.clone();
                if let Some(object) = mappings.as_object_mut() {
                    object.insert(
                        normalized_mac,
                        serde_json::json!({"uart": tty_entry.uart, "power": tty_entry.power}),
                    );
                } else {
                    mappings = serde_json::json!({
                        normalized_mac: {"uart": tty_entry.uart, "power": tty_entry.power}
                    });
                }
                let now = Utc::now();
                device_controller::ActiveModel {
                    device_controller_id: Set(controller.device_controller_id.clone()),
                    mappings: Set(mappings),
                    updated_at: Set(now),
                    ..Default::default()
                }
                .update(&transaction)
                .await?;
            }
            let job = gen5_mapping_queue::Entity::find()
                .filter(Expr::cust_with_values(
                    r#"lower("macAddress") = lower($1)"#,
                    [mac],
                ))
                .filter(gen5_mapping_queue::Column::Status.is_in([
                    gen5_mapping_queue::QueueJobStatus::Waiting,
                    gen5_mapping_queue::QueueJobStatus::FailurePolling,
                ]))
                .order_by_asc(gen5_mapping_queue::Column::CreatedAt)
                .limit(1)
                // SkipLocked: skip rows already claimed by another worker/transaction.
                .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
                .one(&transaction)
                .await?;
            if let Some(job) = job {
                let now = Utc::now();
                gen5_mapping_queue::ActiveModel {
                    id: Set(job.id),
                    status: Set(Some(gen5_mapping_queue::QueueJobStatus::Completed)),
                    last_error: Set(None),
                    failure_dc_ip: Set(None),
                    baseline_mapping_count: Set(None),
                    next_retry_at: Set(None),
                    retry_schedule_index: Set(None),
                    updated_at: Set(now),
                    ..Default::default()
                }
                .update(&transaction)
                .await?;
            }
        }
        CallbackStatus::Failure => {
            let baseline_count = controller
                .mappings
                .as_object()
                .map_or(0_i32, |value| value.len() as i32);
            let job = gen5_mapping_queue::Entity::find()
                .filter(Expr::cust_with_values(
                    r#"lower("macAddress") = lower($1)"#,
                    [mac],
                ))
                .filter(
                    gen5_mapping_queue::Column::Status
                        .eq(gen5_mapping_queue::QueueJobStatus::Waiting),
                )
                .order_by_asc(gen5_mapping_queue::Column::CreatedAt)
                .limit(1)
                // SkipLocked: skip rows already claimed by another worker/transaction.
                .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
                .one(&transaction)
                .await?;
            if let Some(job) = job {
                let now = Utc::now();
                gen5_mapping_queue::ActiveModel {
                    id: Set(job.id),
                    status: Set(Some(gen5_mapping_queue::QueueJobStatus::FailurePolling)),
                    failure_dc_ip: Set(Some(caller_ip.to_owned())),
                    baseline_mapping_count: Set(Some(baseline_count)),
                    next_retry_at: Set(Some(now + Duration::minutes(5))),
                    retry_schedule_index: Set(Some(0)),
                    last_error: Set(Some(
                        "Device controller reported mapping failure".to_owned(),
                    )),
                    updated_at: Set(now),
                    ..Default::default()
                }
                .update(&transaction)
                .await?;
            }
        }
    }
    transaction.commit().await?;
    Ok(())
}

/// PURPOSE: Apply IPL flash confirmation: progress, cancel active test, free device.
/// USE: Flash-confirmation callback handler / DeviceRegistrationRepository.
pub(crate) async fn confirm_flash(
    orm: &DatabaseConnection,
    device_type: &str,
    flashing_status: &IplFlashingStatus,
) -> Result<FlashConfirmationResult, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::callback_store", "confirm_flash");
    info!(
        "\nStarting flash confirmation for device_type: {}, flashing_status: {:?}\n",
        device_type, flashing_status
    );
    let transaction = orm.begin().await?;
    let mut result = FlashConfirmationResult::default();
    let job = device_action_queue::Entity::find()
        .filter(Expr::cust_with_values(
            r#"lower("deviceType") = lower($1)"#,
            [device_type],
        ))
        .filter(device_action_queue::Column::Status.eq(device_action_queue::QueueJobStatus::Waiting))
        .filter(
            Condition::any()
                .add(device_action_queue::Column::IplFlashConfirm.eq(false))
                .add(device_action_queue::Column::IplFlashConfirm.is_null()),
        )
        .order_by_asc(device_action_queue::Column::CreatedAt)
        .limit(1)
        // SkipLocked: skip rows already claimed by another worker/transaction.
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .one(&transaction)
        .await?;
    if let Some(job) = job {
        let job_id = job.id;
        let device_id = job.target_device_id.clone();
        let test_id = job.test_id.clone();
        match flashing_status.status {
            CallbackStatus::Success => {
                info!(
                    "\nFlash confirmation success received for job_id: {}\n",
                    job_id
                );
                if flashing_status.completed {
                    let now = Utc::now();
                    device_action_queue::ActiveModel {
                        id: Set(job_id),
                        ipl_flash_confirm: Set(Some(true)),
                        updated_at: Set(now),
                        ..Default::default()
                    }
                    .update(&transaction)
                    .await?;

                    if let Some(test_id) = test_id.as_deref() {
                        insert_test_log(
                            &transaction,
                            test_id,
                            "Got flash confirmation success, waiting for device to come back online...",
                        )
                        .await?;
                    }
                }

                if let Some(test_id) = test_id.as_deref()
                    && let Some(execution) = active_execution(&transaction, test_id).await?
                {
                    info!(
                        "\nUpdating flash execution progress for test_id: {}, build_id: {}\n",
                        test_id, execution.build_id
                    );
                    result.progress = Some(FlashExecutionProgress {
                        test_id: test_id.to_owned(),
                        build_id: execution.build_id,
                        completed: flashing_status.completed,
                        percentage: flashing_status.percentage,
                        message: flashing_status.message.clone(),
                        created_by: execution
                            .created_by
                            .unwrap_or_else(|| "system".to_owned()),
                    });
                }
            }
            CallbackStatus::Failure => {
                info!(
                    "\nFlash confirmation failure received for job_id: {}\n",
                    job_id
                );
                let now = Utc::now();
                device_action_queue::ActiveModel {
                    id: Set(job_id),
                    ipl_flash_confirm: Set(Some(false)),
                    status: Set(Some(device_action_queue::QueueJobStatus::Cancelled)),
                    updated_at: Set(now),
                    ..Default::default()
                }
                .update(&transaction)
                .await?;
                if let Some(test_id) = test_id.as_deref() {
                    if let Some(execution) = cancel_active_execution(&transaction, test_id).await? {
                        result.cancelled_execution = Some(FlashCancelledExecution {
                            test_id: test_id.to_owned(),
                            build_id: execution.build_id,
                            created_by: execution
                                .created_by
                                .unwrap_or_else(|| "system".to_owned()),
                        });
                    }
                    insert_test_log(
                        &transaction,
                        test_id,
                        "IPL flash confirmation failed. Test execution cancelled",
                    )
                    .await?;
                }
                if let Some(device_id) = device_id {
                    info!("\nReleasing device with device_id: {}\n", device_id);
                    let updated = device::Entity::update_many()
                        .col_expr(
                            device::Column::State,
                            crate::persistence::pg_enum::pg_enum(&device::DeviceState::Free),
                        )
                        .col_expr(device::Column::StateUpdatedAt, Expr::current_timestamp())
                        .col_expr(device::Column::Upgrading, Expr::value(Some(false)))
                        .col_expr(device::Column::Flashing, Expr::value(Some(false)))
                        .col_expr(device::Column::UpdatedAt, Expr::current_timestamp())
                        .filter(device::Column::DeviceId.eq(&device_id))
                        .exec(&transaction)
                        .await?;
                    if updated.rows_affected > 0 {
                        let now = Utc::now();
                        device_state_change::ActiveModel {
                            device_id: Set(device_id.clone()),
                            state: Set(Some(device_state_change::DeviceStateChangeState::Free)),
                            changed_at: Set(now),
                            created_at: Set(now),
                            updated_at: Set(now),
                            ..Default::default()
                        }
                        .insert(&transaction)
                        .await?;
                        result.freed_device_id = Some(device_id);
                    }
                }
            }
        }
    }
    transaction.commit().await?;
    Ok(result)
}

#[derive(Debug, FromQueryResult)]
/// Domain/DTO type `ExecutionProgressRow` used across device stores and handlers.
struct ExecutionProgressRow {
    build_id: String,
    created_by: Option<String>,
}

/// PURPOSE: Implement `active_execution` domain/persistence logic.
/// USE: Called as `active_execution` from handlers, repository impls, or sibling helpers.
async fn active_execution(
    transaction: &impl ConnectionTrait,
    test_id: &str,
) -> Result<Option<ExecutionProgressRow>, DeviceRepositoryError> {
    Ok(test_execution::Entity::find()
        .filter(test_execution::Column::TestId.eq(test_id))
        .filter(test_execution::Column::Status.is_not_in([
            test_execution::TestStatus::Cancelled,
            test_execution::TestStatus::Completed,
            test_execution::TestStatus::Failed,
        ]))
        .select_only()
        .column_as(test_execution::Column::BuildId, "build_id")
        .column_as(test_execution::Column::CreatedBy, "created_by")
        .into_model::<ExecutionProgressRow>()
        .one(transaction)
        .await?)
}

/// PURPOSE: Implement `cancel_active_execution` domain/persistence logic.
/// USE: Called as `cancel_active_execution` from handlers, repository impls, or sibling helpers.
async fn cancel_active_execution(
    transaction: &impl ConnectionTrait,
    test_id: &str,
) -> Result<Option<ExecutionProgressRow>, DeviceRepositoryError> {
    let execution = active_execution(transaction, test_id).await?;
    let Some(execution) = execution else {
        return Ok(None);
    };
    let now = Utc::now();
    test_execution::Entity::update_many()
        .col_expr(
            test_execution::Column::Status,
            crate::persistence::pg_enum::pg_enum(&test_execution::TestStatus::Cancelled),
        )
        .col_expr(
            test_execution::Column::ExecutionPhase,
            crate::persistence::pg_enum::pg_enum(&test_execution::ExecutionPhase::CompleteCancelled),
        )
        .col_expr(test_execution::Column::EndedAt, Expr::value(Some(now)))
        .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
        .filter(test_execution::Column::TestId.eq(test_id))
        .filter(test_execution::Column::Status.is_not_in([
            test_execution::TestStatus::Cancelled,
            test_execution::TestStatus::Completed,
            test_execution::TestStatus::Failed,
        ]))
        .exec(transaction)
        .await?;
    Ok(Some(execution))
}

/// PURPOSE: Create/persist `insert test log`.
/// USE: Create handlers after validation.
async fn insert_test_log(
    transaction: &impl ConnectionTrait,
    test_id: &str,
    message: &str,
) -> Result<(), DeviceRepositoryError> {
    let now = Utc::now();
    log_entry::ActiveModel {
        r#type: Set(log_entry::LogEntryType::Test),
        reference_id: Set(test_id.to_owned()),
        data: Set(serde_json::json!({"message": message})),
        level: Set(log_entry::LogEntryLevel::Info),
        timestamp: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}
