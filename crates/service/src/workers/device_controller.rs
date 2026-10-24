use crate::{
    persistence::entity::{alert, device_controller},
    socket::helpers::controller_timeout_events,
    state::AppState,
    workers::types::ControllerStateTransition,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseBackend, EntityTrait, FromQueryResult, QueryFilter, Set,
    Statement, TransactionTrait,
    sea_query::Expr,
};
use std::sync::Arc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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
    let transaction = match database.orm().begin().await {
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
    #[derive(Debug, FromQueryResult)]
    struct StaleController {
        device_controller_id: String,
    }
    // Subquery last-seen + FOR UPDATE SKIP LOCKED — irreducible.
    let controllers = match StaleController::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT controller."deviceControllerId" AS device_controller_id
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
        [timeout_seconds.into(), batch_size.into()],
    ))
    .all(&transaction)
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
    let now = Utc::now();
    for controller in controllers {
        let controller_id = controller.device_controller_id;
        let update = device_controller::Entity::update_many()
            .col_expr(
                device_controller::Column::State,
                Expr::value(device_controller::DeviceControllerState::NotReachable),
            )
            .col_expr(device_controller::Column::UpdatedAt, Expr::value(now))
            .filter(device_controller::Column::DeviceControllerId.eq(&controller_id))
            .filter(
                device_controller::Column::State
                    .eq(device_controller::DeviceControllerState::Active),
            )
            .exec(&transaction)
            .await;
        if let Err(error) = update {
            let _ = transaction.rollback().await;
            state
                .metrics
                .record_worker_run("controller_heartbeat_timeout", "failure");
            tracing::error!(%error, %controller_id, worker = "controller_heartbeat_timeout", "failed to mark controller unreachable");
            return;
        }
        let alert = alert::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set("ADMIN".to_owned()),
            title: Set("Device Controller Not Reachable".to_owned()),
            message: Set(format!(
                "Device controller {controller_id} changed from online to not reachable"
            )),
            r#type: Set(alert::AlertType::Warning),
            status: Set(alert::AlertStatus::Unread),
            is_read: Set(false),
            device_controller_id: Set(Some(controller_id.clone())),
            data: Set(Some(serde_json::json!({
                "deviceControllerId": controller_id,
                "previousState": "active",
                "state": "not-reachable",
            }))),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&transaction)
        .await;
        match alert {
            Ok(alert) => transitions.push(ControllerStateTransition {
                controller_id,
                alert: controller_alert_json(&alert),
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

fn controller_alert_json(model: &alert::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "user_id": model.user_id,
        "title": model.title,
        "message": model.message,
        "type": match model.r#type {
            alert::AlertType::Info => "info",
            alert::AlertType::Success => "success",
            alert::AlertType::Warning => "warning",
            alert::AlertType::Error => "error",
        },
        "status": match model.status {
            alert::AlertStatus::Unread => "unread",
            alert::AlertStatus::Read => "read",
        },
        "is_read": model.is_read,
        "device_controller_id": model.device_controller_id,
        "data": model.data,
        "created_at": model.created_at,
        "updated_at": model.updated_at,
    })
}
