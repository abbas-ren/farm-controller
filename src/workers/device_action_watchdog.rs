use std::{sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;

use crate::{
    events::{ServerEvent, build_performance_events},
    state::AppState,
};

#[derive(Debug, sqlx::FromRow)]
struct StaleAction {
    id: uuid::Uuid,
    job_type: String,
    test_id: Option<String>,
    release_id: Option<uuid::Uuid>,
    device_id: Option<String>,
    device_type: String,
    user_id: String,
    build_id: Option<String>,
    created_by: Option<String>,
    version: Option<String>,
}

struct ActionTransition {
    events: Vec<ServerEvent>,
}

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
    let mut transaction = match database.pool().begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            record_failure(state, &error, "failed to start watchdog transaction");
            return;
        }
    };
    let timeout_minutes =
        i64::try_from(state.config.workers.device_action_stale_timeout_minutes).unwrap_or(i64::MAX);
    let batch_size = i64::from(state.config.workers.device_action_watchdog_batch_size);
    let actions = match sqlx::query_as::<_, StaleAction>(
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
    )
    .bind(timeout_minutes)
    .bind(batch_size)
    .fetch_all(&mut *transaction)
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
    for action in actions {
        if let Err(error) = sqlx::query(
            r#"UPDATE device_action_queue SET status = 'cancelled', "updatedAt" = now()
               WHERE id = $1"#,
        )
        .bind(action.id)
        .execute(&mut *transaction)
        .await
        {
            let _ = transaction.rollback().await;
            record_failure(state, &error, "failed to cancel stale action");
            return;
        }
        let transition = if action.job_type == "test" {
            match fail_test_action(&mut transaction, action, timeout_minutes).await {
                Ok(transition) => transition,
                Err(error) => {
                    let _ = transaction.rollback().await;
                    record_failure(state, &error, "failed to terminate stale test");
                    return;
                }
            }
        } else {
            match fail_flash_action(&mut transaction, action).await {
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
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    action: StaleAction,
    timeout_minutes: i64,
) -> Result<ActionTransition, sqlx::Error> {
    let Some(test_id) = action.test_id else {
        return Ok(ActionTransition { events: Vec::new() });
    };
    sqlx::query(
        r#"UPDATE test_executions
           SET status = 'failed', "executionPhase" = 'COMPLETE_FAILED',
               "cancelRequested" = true, "cancelRequestedAt" = now(),
               "cancelRequestedBy" = 'System', "cancelHandled" = true,
               "cancelPreserve" = true,
               "cancelSystemReason" = $2,
               "startedAt" = COALESCE("startedAt", now()), "endedAt" = now(),
               "updatedAt" = now()
           WHERE "testId" = $1
             AND status::text NOT IN ('cancelled', 'completed', 'failed')"#,
    )
    .bind(&test_id)
    .bind(format!(
        "stale queue timeout after {timeout_minutes} minute(s)"
    ))
    .execute(&mut **transaction)
    .await?;
    let build_id = action.build_id.unwrap_or_default();
    let owner = action.created_by.unwrap_or(action.user_id);
    Ok(ActionTransition {
        events: test_failure_events(&test_id, &build_id, &owner),
    })
}

async fn fail_flash_action(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    action: StaleAction,
) -> Result<ActionTransition, sqlx::Error> {
    let release_id = match action.release_id {
        Some(release_id) => Some(release_id),
        None => {
            sqlx::query_scalar::<_, uuid::Uuid>(
                r#"SELECT id FROM releases
                   WHERE "testedDeviceId" = $1 AND status::text = 'testing'
                   ORDER BY "updatedAt" DESC LIMIT 1 FOR UPDATE"#,
            )
            .bind(action.device_id.as_deref())
            .fetch_optional(&mut **transaction)
            .await?
        }
    };
    let Some(release_id) = release_id else {
        return Ok(ActionTransition { events: Vec::new() });
    };
    sqlx::query(
        r#"UPDATE releases SET status = 'failed', "isFaulty" = true, "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(release_id)
    .execute(&mut **transaction)
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

fn test_failure_events(test_id: &str, build_id: &str, user_id: &str) -> Vec<ServerEvent> {
    let mut events = vec![
        ServerEvent {
            event: "test_execution".to_owned(),
            payload: serde_json::json!({
                "testId": test_id,
                "type": "status",
                "data": {"status": "failed", "timestamp": chrono::Utc::now().to_rfc3339()},
            }),
            room: Some(format!("user:{user_id}")),
        },
        ServerEvent {
            event: "test_execution_update".to_owned(),
            payload: serde_json::json!({
                "testId": test_id,
                "buildId": build_id,
                "status": "failed",
            }),
            room: Some(format!("build:{build_id}")),
        },
    ];
    events.extend(build_performance_events(build_id));
    events
}

fn build_failure_event(
    release_id: uuid::Uuid,
    device_id: Option<&str>,
    device_type: &str,
    version: Option<&str>,
    user_id: &str,
) -> ServerEvent {
    ServerEvent {
        event: "build_status_changed".to_owned(),
        payload: serde_json::json!({
            "releaseId": release_id,
            "status": "failed",
            "isFaulty": true,
            "deviceId": device_id,
            "deviceType": device_type,
            "version": version,
            "userId": user_id,
        }),
        room: None,
    }
}

fn record_failure(state: &AppState, error: &sqlx::Error, message: &str) {
    state
        .metrics
        .record_worker_run("device_action_watchdog", "failure");
    tracing::error!(%error, worker = "device_action_watchdog", "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_test_events_refresh_owner_and_build_views() {
        let events = test_failure_events("test-1", "build-1", "user-1");
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].event, "test_execution");
        assert_eq!(events[0].room.as_deref(), Some("user:user-1"));
        assert_eq!(events[1].event, "test_execution_update");
        assert_eq!(events[1].payload["status"], "failed");
        assert_eq!(events[1].room.as_deref(), Some("build:build-1"));
        assert_eq!(events[2].room.as_deref(), Some("dashboard:admin"));
        assert_eq!(events[3].room.as_deref(), Some("dashboard:user"));
        assert_eq!(events[4].room.as_deref(), Some("build:build-1"));
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
        assert!(event.room.is_none());
    }
}
