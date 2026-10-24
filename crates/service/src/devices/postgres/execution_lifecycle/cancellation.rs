//! Request cancellation, terminalize executions, and free devices.
//!
//! Ports Node `TestService` cancel paths including NFS cleanup metadata.

use super::*;

use crate::persistence::entity::{
    device, device_action_queue, device_state_change, fallback_update, log_entry, test_execution,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, JoinType, QueryFilter, QuerySelect, QueryTrait,
    RelationTrait, Set, TransactionTrait,
    sea_query::{Alias, Expr, LockType},
};

impl PostgresDeviceRepository {
    /// PURPOSE: Implement `request_test_cancellation_inner` domain/persistence logic.
    /// USE: Called as `request_test_cancellation_inner` from handlers, repository impls, or sibling helpers.
    pub(super) async fn request_test_cancellation_inner(
        &self,
        test_id: &str,
        user_id: &str,
        nfs_host_path: &str,
    ) -> Result<Option<TestCancellationResult>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::cancellation", "request_test_cancellation_inner");
        let transaction = self.orm().begin().await?;

        let mut cancel_query = test_execution::Entity::find()
            .filter(test_execution::Column::TestId.eq(test_id))
            .join(
                JoinType::LeftJoin,
                test_execution::Relation::Device.def(),
            )
            .select_only()
            .column(test_execution::Column::CancelRequested)
            .column(test_execution::Column::Status)
            .column(test_execution::Column::BuildId)
            .column(test_execution::Column::ExecutionPhase)
            .column(test_execution::Column::DeviceId)
            .column(test_execution::Column::DeviceType)
            .column(test_execution::Column::BuildVersion)
            .column(test_execution::Column::TestCycleId)
            .column(test_execution::Column::CreatedBy)
            .column_as(device::Column::IpAddress, "device_ip")
            .column_as(device::Column::NfsPath, "device_nfs_path");
        // FOR UPDATE OF execution only — Postgres rejects locking the nullable LEFT JOIN side.
        QueryTrait::query(&mut cancel_query).lock_with_tables(
            LockType::Update,
            [Alias::new("test_executions")],
        );
        let row = cancel_query
            .into_tuple::<(
                Option<bool>,
                Option<test_execution::TestStatus>,
                String,
                Option<test_execution::ExecutionPhase>,
                Option<String>,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            )>()
            .one(&transaction)
            .await?;

        let Some((
            requested,
            status,
            build_id,
            phase,
            device_id,
            device_type,
            build_version,
            test_cycle_id,
            created_by,
            device_ip,
            device_nfs_path,
        )) = row
        else {
            transaction.rollback().await?;
            return Ok(None);
        };

        let requested = requested.unwrap_or(false);
        let status = status.unwrap_or(test_execution::TestStatus::NotExecuted);
        let status_text = match status {
            test_execution::TestStatus::NotExecuted => "not_executed",
            test_execution::TestStatus::InProgress => "in_progress",
            test_execution::TestStatus::Completed => "completed",
            test_execution::TestStatus::Queued => "queued",
            test_execution::TestStatus::Cancelled => "cancelled",
            test_execution::TestStatus::Failed => "failed",
        };
        let changed = !requested && !matches!(status_text, "cancelled" | "completed" | "failed");
        let phase = phase.unwrap_or(test_execution::ExecutionPhase::PrepareArtifacts);
        let phase_text = execution_phase_str(phase);
        // Always terminalize on cancel (including IPL/boot wait). Deferred cancel left the DUT
        // BUSY until heartbeat; cancel must free the device for other executions immediately.
        let already_finalizing = matches!(
            phase,
            test_execution::ExecutionPhase::SendFallbackFlashRequest
                | test_execution::ExecutionPhase::WaitForFallbackCompletion
                | test_execution::ExecutionPhase::CompleteSuccess
                | test_execution::ExecutionPhase::CompleteFailed
                | test_execution::ExecutionPhase::CompleteCancelled
        );
        let terminalized = changed && !already_finalizing;
        let now = chrono::Utc::now();
        let mut freed_device_id = None;

        if changed {
            test_execution::Entity::update_many()
                .col_expr(
                    test_execution::Column::CancelRequested,
                    Expr::value(Some(true)),
                )
                .col_expr(
                    test_execution::Column::CancelRequestedAt,
                    Expr::value(Some(now)),
                )
                .col_expr(
                    test_execution::Column::CancelRequestedBy,
                    Expr::value(Some(user_id.to_owned())),
                )
                .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
                .filter(test_execution::Column::TestId.eq(test_id))
                .exec(&transaction)
                .await?;

            log_entry::ActiveModel {
                r#type: Set(log_entry::LogEntryType::Test),
                reference_id: Set(test_id.to_owned()),
                data: Set(serde_json::json!({
                    "message": format!("Cancel requested during phase: {phase_text}")
                })),
                level: Set(log_entry::LogEntryLevel::Info),
                timestamp: Set(Some(now)),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(&transaction)
            .await?;
        }

        if terminalized {
            test_execution::Entity::update_many()
                .col_expr(
                    test_execution::Column::Status,
                    crate::persistence::pg_enum::pg_enum(&test_execution::TestStatus::Cancelled),
                )
                .col_expr(
                    test_execution::Column::CancelHandled,
                    Expr::value(Some(true)),
                )
                .col_expr(
                    test_execution::Column::ExecutionPhase,
                    crate::persistence::pg_enum::pg_enum(&test_execution::ExecutionPhase::CompleteCancelled),
                )
                .col_expr(
                    test_execution::Column::StartedAt,
                    Expr::cust("COALESCE(\"startedAt\", now())"),
                )
                .col_expr(test_execution::Column::EndedAt, Expr::value(Some(now)))
                .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
                .filter(test_execution::Column::TestId.eq(test_id))
                .filter(
                    test_execution::Column::Status.is_not_in([
                        test_execution::TestStatus::Cancelled,
                        test_execution::TestStatus::Completed,
                        test_execution::TestStatus::Failed,
                    ]),
                )
                .exec(&transaction)
                .await?;

            // Do not enqueue fallback flash on cancel — that would keep the DUT BUSY.
            // Cancel any in-flight fallback for this test so heartbeat treats the device as free.
            let _ = nfs_host_path;
            let _ = build_version;
            fallback_update::Entity::update_many()
                .col_expr(
                    fallback_update::Column::Status,
                    crate::persistence::pg_enum::pg_enum(&fallback_update::FallbackStatus::Failed),
                )
                .col_expr(fallback_update::Column::UpdatedAt, Expr::value(now))
                .filter(fallback_update::Column::TestId.eq(test_id))
                .filter(fallback_update::Column::Status.is_in([
                    fallback_update::FallbackStatus::Pending,
                    fallback_update::FallbackStatus::Flashing,
                ]))
                .exec(&transaction)
                .await?;

            device_action_queue::Entity::update_many()
                .col_expr(
                    device_action_queue::Column::Status,
                    crate::persistence::pg_enum::pg_enum(&device_action_queue::QueueJobStatus::Cancelled),
                )
                .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
                .filter(device_action_queue::Column::TestId.eq(test_id))
                .filter(
                    device_action_queue::Column::Status.is_not_in([
                        device_action_queue::QueueJobStatus::Completed,
                        device_action_queue::QueueJobStatus::Cancelled,
                    ]),
                )
                .exec(&transaction)
                .await?;

            if let Some(assigned_device_id) = device_id.as_deref().filter(|id| !id.is_empty()) {
                device::Entity::update_many()
                    .col_expr(
                        device::Column::State,
                        crate::persistence::pg_enum::pg_enum(&device::DeviceState::Free),
                    )
                    .col_expr(device::Column::StateUpdatedAt, Expr::value(Some(now)))
                    .col_expr(device::Column::Upgrading, Expr::value(Some(false)))
                    .col_expr(device::Column::Flashing, Expr::value(Some(false)))
                    .col_expr(device::Column::UpdatedAt, Expr::value(now))
                    .filter(device::Column::DeviceId.eq(assigned_device_id))
                    .filter(device::Column::DeletedAt.is_null())
                    .exec(&transaction)
                    .await?;
                device_state_change::ActiveModel {
                    device_id: Set(assigned_device_id.to_owned()),
                    state: Set(Some(device_state_change::DeviceStateChangeState::Free)),
                    changed_at: Set(now),
                    created_at: Set(now),
                    updated_at: Set(now),
                    ..Default::default()
                }
                .insert(&transaction)
                .await?;
                freed_device_id = Some(assigned_device_id.to_owned());
            }

            log_entry::ActiveModel {
                r#type: Set(log_entry::LogEntryType::Test),
                reference_id: Set(test_id.to_owned()),
                data: Set(serde_json::json!({
                    "message": "This test has been cancelled successfully"
                })),
                level: Set(log_entry::LogEntryLevel::Info),
                timestamp: Set(Some(now)),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(&transaction)
            .await?;
        }

        transaction.commit().await?;
        // Notify eagent whenever a DUT may still be running or mid-boot for this test.
        let notify_device = matches!(
            phase,
            test_execution::ExecutionPhase::StartTestExecution
                | test_execution::ExecutionPhase::WaitForDeviceBoot
                | test_execution::ExecutionPhase::SendFlashRequest
                | test_execution::ExecutionPhase::DeviceAssigned
                | test_execution::ExecutionPhase::WaitForIplConfirmation
                | test_execution::ExecutionPhase::SendIplRequest
        );
        Ok(Some(TestCancellationResult {
            build_id,
            created_by: created_by.unwrap_or_else(|| user_id.to_owned()),
            cleanup_device_type: terminalized
                .then_some(phase)
                .filter(|phase| {
                    matches!(
                        phase,
                        test_execution::ExecutionPhase::PrepareArtifacts
                            | test_execution::ExecutionPhase::PrepareTestScripts
                    )
                })
                .map(|_| device_type.clone()),
            device_type: Some(device_type),
            device_nfs_path,
            device_id: freed_device_id,
            device_ip: notify_device.then_some(device_ip).flatten(),
            phase: phase_text.to_owned(),
            changed,
            terminalized,
            test_cycle_id,
        }))
    }
}

/// PURPOSE: Serialize `execution phase str` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
fn execution_phase_str(phase: test_execution::ExecutionPhase) -> &'static str {
    let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::cancellation", "execution_phase_str");
    match phase {
        test_execution::ExecutionPhase::PrepareArtifacts => "PREPARE_ARTIFACTS",
        test_execution::ExecutionPhase::PrepareTestScripts => "PREPARE_TEST_SCRIPTS",
        test_execution::ExecutionPhase::QueueTestExecution => "QUEUE_TEST_EXECUTION",
        test_execution::ExecutionPhase::ProcessingQueued => "PROCESSING_QUEUED",
        test_execution::ExecutionPhase::AssigningDevice => "ASSIGNING_DEVICE",
        test_execution::ExecutionPhase::DeviceAssigned => "DEVICE_ASSIGNED",
        test_execution::ExecutionPhase::SendIplRequest => "SEND_IPL_REQUEST",
        test_execution::ExecutionPhase::WaitForIplConfirmation => "WAIT_FOR_IPL_CONFIRMATION",
        test_execution::ExecutionPhase::RequeueAfterIplConfirmation => {
            "REQUEUE_AFTER_IPL_CONFIRMATION"
        }
        test_execution::ExecutionPhase::SendFlashRequest => "SEND_FLASH_REQUEST",
        test_execution::ExecutionPhase::WaitForDeviceBoot => "WAIT_FOR_DEVICE_BOOT",
        test_execution::ExecutionPhase::StartTestExecution => "START_TEST_EXECUTION",
        test_execution::ExecutionPhase::SendFallbackFlashRequest => "SEND_FALLBACK_FLASH_REQUEST",
        test_execution::ExecutionPhase::WaitForFallbackCompletion => "WAIT_FOR_FALLBACK_COMPLETION",
        test_execution::ExecutionPhase::CompleteSuccess => "COMPLETE_SUCCESS",
        test_execution::ExecutionPhase::CompleteFailed => "COMPLETE_FAILED",
        test_execution::ExecutionPhase::CompleteCancelled => "COMPLETE_CANCELLED",
    }
}
