use crate::{
    socket::helpers::controller_timeout_events, state::AppState,
    workers::types::ControllerStateTransition,
};
use std::sync::Arc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

pub(crate) async fn controller_heartbeat_timeout_worker(
    state: Arc<AppState>,
    cancellation: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.controller_heartbeat_poll_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_controller_heartbeat_timeout_cycle(&state).await,
        }
    }
    tracing::info!(worker = "controller_heartbeat_timeout", "worker stopped");
}

async fn run_controller_heartbeat_timeout_cycle(state: &AppState) {
    let Some(database) = &state.database else {
        return;
    };
    let mut transaction = match database.pool().begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            state
                .metrics
                .record_worker_run("controller_heartbeat_timeout", "failure");
            tracing::error!(%error, worker = "controller_heartbeat_timeout", "failed to start timeout transaction");
            return;
        }
    };
    let timeout_seconds = i64::try_from(state.config.workers.controller_heartbeat_timeout_seconds)
        .unwrap_or(i64::MAX);
    let batch_size = i64::from(state.config.workers.controller_heartbeat_batch_size);
    let controllers = match sqlx::query_scalar::<_, String>(
        r#"SELECT controller."deviceControllerId"
           FROM device_controllers controller
           WHERE controller."deletedAt" IS NULL
             AND controller.state::text = 'active'
             AND COALESCE(
                   (SELECT max(heartbeat.timestamp)
                    FROM device_controller_heartbeats heartbeat
                    WHERE heartbeat."controllerId" = controller."deviceControllerId"),
                   controller."updatedAt", controller."createdAt"
                 ) < now() - ($1 * interval '1 second')
           ORDER BY controller."updatedAt" ASC
           LIMIT $2
           FOR UPDATE OF controller SKIP LOCKED"#,
    )
    .bind(timeout_seconds)
    .bind(batch_size)
    .fetch_all(&mut *transaction)
    .await
    {
        Ok(controllers) => controllers,
        Err(error) => {
            let _ = transaction.rollback().await;
            state
                .metrics
                .record_worker_run("controller_heartbeat_timeout", "failure");
            tracing::error!(%error, worker = "controller_heartbeat_timeout", "failed to load stale controllers");
            return;
        }
    };
    let mut transitions = Vec::with_capacity(controllers.len());
    for controller_id in controllers {
        let update = sqlx::query(
            r#"UPDATE device_controllers
               SET state = 'not-reachable', "updatedAt" = now()
               WHERE "deviceControllerId" = $1 AND state::text = 'active'"#,
        )
        .bind(&controller_id)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = update {
            let _ = transaction.rollback().await;
            state
                .metrics
                .record_worker_run("controller_heartbeat_timeout", "failure");
            tracing::error!(%error, %controller_id, worker = "controller_heartbeat_timeout", "failed to mark controller unreachable");
            return;
        }
        let alert = sqlx::query_scalar::<_, serde_json::Value>(
            r#"INSERT INTO alerts
                  (id, user_id, title, message, type, status, is_read,
                   device_controller_id, data, created_at, updated_at)
               VALUES (gen_random_uuid(), 'ADMIN', 'Device Controller Not Reachable',
                       'Device controller ' || $1 || ' changed from online to not reachable',
                       'warning', 'unread', false, $1,
                       jsonb_build_object('deviceControllerId', $1,
                                          'previousState', 'active',
                                          'state', 'not-reachable'),
                       now(), now())
               RETURNING to_jsonb(alerts)"#,
        )
        .bind(&controller_id)
        .fetch_one(&mut *transaction)
        .await;
        match alert {
            Ok(alert) => transitions.push(ControllerStateTransition {
                controller_id,
                alert,
            }),
            Err(error) => {
                let _ = transaction.rollback().await;
                state
                    .metrics
                    .record_worker_run("controller_heartbeat_timeout", "failure");
                tracing::error!(%error, %controller_id, worker = "controller_heartbeat_timeout", "failed to persist controller alert");
                return;
            }
        }
    }
    if let Err(error) = transaction.commit().await {
        state
            .metrics
            .record_worker_run("controller_heartbeat_timeout", "failure");
        tracing::error!(%error, worker = "controller_heartbeat_timeout", "failed to commit controller timeouts");
        return;
    }
    for transition in &transitions {
        for event in controller_timeout_events(transition) {
            if let Err(error) = state.event_publisher.publish(event) {
                tracing::warn!(%error, controller_id = transition.controller_id, "failed to publish controller timeout");
            }
        }
    }
    state
        .metrics
        .record_worker_items("controller_heartbeat_timeout", transitions.len() as u64);
    state
        .metrics
        .record_worker_run("controller_heartbeat_timeout", "success");
}
