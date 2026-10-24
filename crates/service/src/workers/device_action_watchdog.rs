use std::{sync::Arc, time::Duration};

use chrono::Utc;
use sea_orm::{
    ColumnTrait, DatabaseBackend, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Statement, TransactionTrait,
    sea_query::{Expr, LockType},
};
use tokio_util::sync::CancellationToken;

use crate::{
    persistence::entity::{device_action_queue, release, test_execution},
    socket::helpers::{build_failure_event, test_failure_events},
    state::AppState,
    workers::types::{ActionTransition, StaleAction},
};

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
    // Join + interval timeout + SKIP LOCKED — keep as Statement.
    let actions = match StaleAction::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT action.id, action.type::text AS job_type, action."testId" AS test_id,
                  action."releaseId" AS release_id, action."targetDeviceId" AS device_id,
                  action."deviceType" AS device_type, action."userId" AS user_id,
                  test."buildId"::text AS build_id, test."createdBy" AS created_by,
                  release.version
           FROM device_action_queue action
           LEFT JOIN test_executions test ON test."testId" = action."testId"
           LEFT JOIN releases release ON release.id = action."releaseId"
           WHERE action.status::text IN ('running', 'waiting')
             AND action."updatedAt" < now() - ($1 * interval '1 minute')
           ORDER BY action."updatedAt" ASC
           LIMIT $2
           FOR UPDATE OF action SKIP LOCKED"#,
        [timeout_minutes.into(), (batch_size as i64).into()],
    ))
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
                Expr::value(device_action_queue::QueueJobStatus::Cancelled),
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
            Expr::value(Some(test_execution::TestStatus::Failed)),
        )
        .col_expr(
            test_execution::Column::ExecutionPhase,
            Expr::value(Some(test_execution::ExecutionPhase::CompleteFailed)),
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
                .lock(LockType::Update)
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
            Expr::value(Some(release::ReleaseTestStatus::Failed)),
        )
        .col_expr(release::Column::IsFaulty, Expr::value(Some(true)))
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
    fn stale_test_events_refresh_owner_and_build_views() {
        let events = test_failure_events("test-1", "build-1", "user-1");
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].event, "test_execution");
        assert_eq!(events[1].event, "test_execution_update");
        assert_eq!(events[1].payload["status"], "failed");
    }

    #[test]
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
