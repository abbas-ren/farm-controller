//! Stale device-action watchdog for jobs stuck in running/waiting.
//!
//! Cancels or fails actions (and linked tests) whose `updated_at` exceeds the
//! configured stale timeout, frees devices, and emits farm WebSocket events.

use std::{sync::Arc, time::Duration};

use chrono::Utc;
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
    RelationTrait, TransactionTrait,
    sea_query::{Alias, Expr, LockBehavior, LockType},
};
use tokio_util::sync::CancellationToken;

use crate::{
    persistence::entity::{device_action_queue, release, test_execution},
    socket::helpers::{build_failure_event, test_failure_events},
    state::AppState,
    workers::types::{ActionTransition, StaleAction},
};

/// Poll loop that fails stale running/waiting device-action jobs.
pub(super) async fn worker(state: Arc<AppState>, cancellation: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.device_action_watchdog_poll_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_cycle(&state).await,
        }
    }
    tracing::info!(worker = "device_action_watchdog", "worker stopped");
}

/// Find stale actions past the timeout, transition each, and publish events.
async fn run_cycle(state: &AppState) {
    let Some(database) = &state.database else {
        return;
    };
    let transaction = match database.orm().begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            record_failure(state, &error, "failed to start watchdog transaction");
            return;
        }
    };
    let timeout_minutes =
        i64::try_from(state.config.workers.device_action_stale_timeout_minutes).unwrap_or(i64::MAX);
    let batch_size = u64::from(state.config.workers.device_action_watchdog_batch_size);
    let mut stale_query = device_action_queue::Entity::find()
        .join(
            JoinType::LeftJoin,
            device_action_queue::Relation::TestExecution.def(),
        )
        .join(
            JoinType::LeftJoin,
            device_action_queue::Relation::Release.def(),
        )
        .select_only()
        // StaleAction maps camelCase queue columns via #[sea_orm(alias = "...")].
        .column(device_action_queue::Column::Id)
        .column_as(Expr::cust(r#""device_action_queue"."type"::text"#), "job_type")
        .column(device_action_queue::Column::TestId)
        .column(device_action_queue::Column::ReleaseId)
        .column_as(device_action_queue::Column::TargetDeviceId, "device_id")
        .column(device_action_queue::Column::DeviceType)
        .column(device_action_queue::Column::UserId)
        .column_as(Expr::cust(r#"test_executions."buildId"::text"#), "build_id")
        .column_as(test_execution::Column::CreatedBy, "created_by")
        .column_as(release::Column::Version, "version")
        .filter(
            device_action_queue::Column::Status.is_in([
                device_action_queue::QueueJobStatus::Running,
                device_action_queue::QueueJobStatus::Waiting,
            ]),
        )
        .filter(Expr::cust_with_values(
            r#""device_action_queue"."updatedAt" < now() - ($1 * interval '1 minute')"#,
            [timeout_minutes],
        ))
        .order_by_asc(device_action_queue::Column::UpdatedAt)
        .limit(batch_size);
    // FOR UPDATE OF queue only — Postgres rejects locking the nullable LEFT JOIN side.
    QueryTrait::query(&mut stale_query).lock_with_tables_behavior(
        LockType::Update,
        [Alias::new("device_action_queue")],
        LockBehavior::SkipLocked,
    );
    let actions = match stale_query
        .into_model::<StaleAction>()
        .all(&transaction)
        .await
    {
        Ok(actions) => actions,
        Err(error) => {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to load stale actions");
            return;
        }
    };

    let mut transitions = Vec::with_capacity(actions.len());
    let now = Utc::now();
    for action in actions {
        if let Err(error) = device_action_queue::Entity::update_many()
            .col_expr(
                device_action_queue::Column::Status,
                crate::persistence::pg_enum::pg_enum(&device_action_queue::QueueJobStatus::Cancelled),
            )
            .col_expr(device_action_queue::Column::UpdatedAt, Expr::value(now))
            .filter(device_action_queue::Column::Id.eq(action.id))
            .exec(&transaction)
            .await
        {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to cancel stale action");
            return;
        }
        let transition = if action.job_type == "test" {
            match fail_test_action(&transaction, action, timeout_minutes).await {
                Ok(transition) => transition,
                Err(error) => {
                    let _ = transaction.rollback().await;
                    record_failure(state, &error, "failed to terminate stale test");
                    return;
                }
            }
        } else {
            match fail_flash_action(&transaction, action).await {
                Ok(transition) => transition,
                Err(error) => {
                    let _ = transaction.rollback().await;
                    record_failure(state, &error, "failed to terminate stale flash");
                    return;
                }
            }
        };
        transitions.push(transition);
    }
    if let Err(error) = transaction.commit().await {
        record_failure(state, &error, "failed to commit stale action cleanup");
        return;
    }
    for transition in &transitions {
        for event in &transition.events {
            if let Err(error) = state.event_publisher.publish(event.clone()) {
                tracing::warn!(%error, "failed to publish stale action transition");
            }
        }
    }
    state
        .metrics
        .record_worker_items("device_action_watchdog", transitions.len() as u64);
    state
        .metrics
        .record_worker_run("device_action_watchdog", "success");
}

/// Fail a stale TEST job: cancel queue row, fail execution, free device, emit events.
async fn fail_test_action(
    transaction: &sea_orm::DatabaseTransaction,
    action: StaleAction,
    timeout_minutes: i64,
) -> Result<ActionTransition, sea_orm::DbErr> {
    let Some(test_id) = action.test_id else {
        return Ok(ActionTransition { events: Vec::new() });
    };
    let now = Utc::now();
    let reason = format!("stale queue timeout after {timeout_minutes} minute(s)");
    test_execution::Entity::update_many()
        .col_expr(
            test_execution::Column::Status,
            crate::persistence::pg_enum::pg_enum(&test_execution::TestStatus::Failed),
        )
        .col_expr(
            test_execution::Column::ExecutionPhase,
            crate::persistence::pg_enum::pg_enum(&test_execution::ExecutionPhase::CompleteFailed),
        )
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
            Expr::value(Some("System".to_owned())),
        )
        .col_expr(
            test_execution::Column::CancelHandled,
            Expr::value(Some(true)),
        )
        .col_expr(
            test_execution::Column::CancelPreserve,
            Expr::value(Some(true)),
        )
        .col_expr(
            test_execution::Column::CancelSystemReason,
            Expr::value(Some(reason)),
        )
        .col_expr(
            test_execution::Column::StartedAt,
            Expr::cust(r#"COALESCE("startedAt", now())"#),
        )
        .col_expr(test_execution::Column::EndedAt, Expr::value(Some(now)))
        .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
        .filter(test_execution::Column::TestId.eq(&test_id))
        .filter(test_execution::Column::Status.is_not_in([
            test_execution::TestStatus::Cancelled,
            test_execution::TestStatus::Completed,
            test_execution::TestStatus::Failed,
        ]))
        .exec(transaction)
        .await?;
    let build_id = action.build_id.unwrap_or_default();
    let owner = action.created_by.unwrap_or(action.user_id);
    Ok(ActionTransition {
        events: test_failure_events(&test_id, &build_id, &owner),
    })
}

/// Fail a stale FLASH job and clear device busy flags.
async fn fail_flash_action(
    transaction: &sea_orm::DatabaseTransaction,
    action: StaleAction,
) -> Result<ActionTransition, sea_orm::DbErr> {
    let release_id = match action.release_id {
        Some(release_id) => Some(release_id),
        None => {
            release::Entity::find()
                .filter(release::Column::TestedDeviceId.eq(action.device_id.as_deref()))
                .filter(release::Column::Status.eq(release::ReleaseTestStatus::Testing))
                .order_by_desc(release::Column::UpdatedAt)
                .limit(1)
                .lock(sea_orm::sea_query::LockType::Update)
                .select_only()
                .column(release::Column::Id)
                .into_tuple::<uuid::Uuid>()
                .one(transaction)
                .await?
        }
    };
    let Some(release_id) = release_id else {
        return Ok(ActionTransition { events: Vec::new() });
    };
    let now = Utc::now();
    release::Entity::update_many()
        .col_expr(
            release::Column::Status,
            crate::persistence::pg_enum::pg_enum(&release::ReleaseTestStatus::Failed),
        )
        .col_expr(release::Column::IsFaulty, Expr::value(true))
        .col_expr(release::Column::UpdatedAt, Expr::value(now))
        .filter(release::Column::Id.eq(release_id))
        .exec(transaction)
        .await?;
    Ok(ActionTransition {
        events: vec![build_failure_event(
            release_id,
            action.device_id.as_deref(),
            &action.device_type,
            action.version.as_deref(),
            &action.user_id,
        )],
    })
}

/// Record watchdog cycle failure metrics.
fn record_failure(state: &AppState, error: &sea_orm::DbErr, message: &str) {
    state
        .metrics
        .record_worker_run("device_action_watchdog", "failure");
    tracing::error!(%error, worker = "device_action_watchdog", "{message}");
}

#[cfg(test)]
mod tests {
    use crate::socket::helpers::{build_failure_event, test_failure_events};

    #[test]
    /// Unit test: stale TEST watchdog events match UI contracts.
    fn stale_test_events_refresh_owner_and_build_views() {
        let events = test_failure_events("test-1", "build-1", "user-1");
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].event, "test_execution");
        assert_eq!(events[1].event, "test_execution_update");
        assert_eq!(events[1].payload["status"], "failed");
    }

    #[test]
    /// Unit test: stale FLASH event shape for upload listeners.
    fn stale_flash_event_preserves_upload_listener_contract() {
        let release_id = uuid::Uuid::nil();
        let event =
            build_failure_event(release_id, Some("device-1"), "x5h", Some("1.0.0"), "user-1");
        assert_eq!(event.event, "build_status_changed");
        assert_eq!(event.payload["releaseId"], release_id.to_string());
        assert_eq!(event.payload["status"], "failed");
        assert_eq!(event.payload["isFaulty"], true);
    }
}
