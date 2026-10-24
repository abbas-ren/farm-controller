use std::{sync::Arc, time::Duration};

use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    devices::{EDGE_CONTROLLER_PORT, RELAY_CONTROLLER_TIMEOUT, RelayControllerAction},
    persistence::entity::pending_relay_config,
    socket::{
        helpers::relay_timeout_event,
        types::{
            farm::{EventHub, ServerEvent},
            frontend::FrontendMessagePayload,
        },
    },
    state::AppState,
    workers::types::{ExpiredRelayConfiguration, RelaySyncJob, RelaySyncService},
};

use sea_orm::{
    ColumnTrait, DatabaseBackend, EntityTrait, FromQueryResult, QueryFilter, Statement,
    TransactionTrait,
    sea_query::Expr,
};

impl RelaySyncService {
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, receiver) = mpsc::channel(capacity);
        Self {
            sender,
            receiver: Arc::new(Mutex::new(Some(receiver))),
        }
    }

    pub(crate) fn enqueue(
        &self,
        actions: Vec<RelayControllerAction>,
        relay_id: Option<uuid::Uuid>,
    ) -> Result<(), String> {
        self.sender
            .try_send(RelaySyncJob { actions, relay_id })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    "relay synchronization queue is full".to_owned()
                }
                mpsc::error::TrySendError::Closed(_) => {
                    "relay synchronization worker is unavailable".to_owned()
                }
            })
    }

    pub(crate) fn start_worker(
        &self,
        cancellation: CancellationToken,
        event_publisher: Arc<EventHub>,
    ) -> JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            let Some(mut receiver) = service.receiver.lock().await.take() else {
                return;
            };
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    job = receiver.recv() => match job {
                        Some(job) => dispatch_relay_actions(job, &event_publisher).await,
                        None => break,
                    },
                }
            }
            tracing::info!(worker = "relay_sync", "worker stopped");
        })
    }
}

async fn dispatch_relay_actions(job: RelaySyncJob, event_publisher: &EventHub) {
    let total = job.actions.len();
    let has_config_actions = job
        .actions
        .iter()
        .any(|action| matches!(action, RelayControllerAction::Configure { .. }));
    publish_relay_status(event_publisher, job.relay_id, "in_progress", total, 0, &[]);
    let mut completed = 0;
    let mut errors = Vec::new();
    for action in job.actions {
        let (controller_address, path, payload, timeout) = match action {
            RelayControllerAction::Remove {
                controller_address,
                device_mac,
                relay_serial,
                channel_number,
            } => (
                controller_address,
                "relay/delete",
                serde_json::json!({
                    "mac": device_mac.to_lowercase(),
                    "serial": relay_serial,
                    "channel": channel_number,
                }),
                RELAY_CONTROLLER_TIMEOUT,
            ),
            RelayControllerAction::Configure {
                controller_address,
                device_mac,
                relay_serial,
                channel_number,
                device_generation,
            } => (
                controller_address,
                "relay/config",
                serde_json::json!({
                    "mac": device_mac.to_lowercase(),
                    "serial": relay_serial,
                    "channel": channel_number,
                    "gen": device_generation.as_deref().and_then(device_generation_number),
                }),
                RELAY_CONTROLLER_TIMEOUT,
            ),
        };
        let Ok(url) = reqwest::Url::parse(&format!(
            "http://{controller_address}:{EDGE_CONTROLLER_PORT}/{path}"
        )) else {
            let error = format!("Invalid controller address for {path}: {controller_address}");
            tracing::warn!(%controller_address, path, "invalid controller address for relay synchronization");
            errors.push(error);
            continue;
        };
        let client = match crate::external_http::client(timeout) {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, path, "failed to create relay synchronization client");
                errors.push(format!("Failed to initialize {path}: {error}"));
                continue;
            }
        };
        match client.post(url).json(&payload).send().await {
            Ok(response) if response.status().is_success() => completed += 1,
            Ok(response) => {
                let status = response.status();
                let detail = response
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|body| {
                        body.get("error")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    });
                tracing::warn!(%status, path, error = ?detail, "controller rejected relay synchronization");
                errors.push(match detail {
                    Some(detail) => {
                        format!("Controller rejected {path} with {status}: {detail}")
                    }
                    None => format!("Controller rejected {path} with {status}"),
                });
            }
            Err(error) => {
                tracing::warn!(%error, path, "relay synchronization request did not complete");
                errors.push(format!("{path} request failed: {error}"));
            }
        }
    }
    if !errors.is_empty() {
        publish_relay_status(
            event_publisher,
            job.relay_id,
            if completed == 0 {
                "failed"
            } else {
                "completed_with_errors"
            },
            total,
            completed,
            &errors,
        );
    } else if !has_config_actions {
        publish_relay_status(
            event_publisher,
            job.relay_id,
            "completed",
            total,
            completed,
            &[],
        );
    }
}

fn publish_relay_status(
    event_publisher: &EventHub,
    relay_id: Option<uuid::Uuid>,
    status: &str,
    total: usize,
    completed: usize,
    errors: &[String],
) {
    let message = match status {
        "failed" => errors
            .first()
            .map(String::as_str)
            .or(Some("Relay hardware synchronization failed")),
        "completed_with_errors" => Some("Relay hardware synchronization completed with errors"),
        _ => None,
    };
    let event = ServerEvent::frontend(FrontendMessagePayload::RelayConfigurationStatus {
        relay_id,
        controller_id: None,
        status: status.to_owned(),
        total,
        completed,
        errors: errors.to_vec(),
        message: message.map(str::to_owned),
    });

    if let Err(error) = event_publisher.publish(event) {
        tracing::warn!(%error, status, "failed to publish relay synchronization status");
    }
}

fn device_generation_number(value: &str) -> Option<u32> {
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

pub(crate) async fn relay_configuration_timeout_worker(
    state: Arc<AppState>,
    cancellation: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.relay_config_poll_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_relay_configuration_timeout_cycle(&state).await,
        }
    }
    tracing::info!(worker = "relay_config_timeout", "worker stopped");
}

async fn run_relay_configuration_timeout_cycle(state: &AppState) {
    let Some(database) = &state.database else {
        return;
    };
    let transaction = match database.orm().begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            state
                .metrics
                .record_worker_run("relay_config_timeout", "failure");
            tracing::error!(%error, worker = "relay_config_timeout", "failed to start timeout transaction");
            return;
        }
    };
    let timeout_seconds =
        i64::try_from(state.config.workers.relay_config_timeout_seconds).unwrap_or(i64::MAX);
    let batch_size = i64::from(state.config.workers.relay_config_batch_size);
    // Join pending configs with relay channel/relay + FOR UPDATE SKIP LOCKED.
    let expired = match ExpiredRelayConfiguration::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT pending.id, pending."deviceMac" AS device_mac,
                  pending."channelNo" AS channel_number,
                  relay.id AS relay_id,
                  relay."deviceControllerId" AS controller_id
           FROM pending_relay_configs pending
           LEFT JOIN relay_channels channel ON channel.id = pending."relayChannelId"
           LEFT JOIN relays relay ON relay.id = channel."relayId"
           WHERE pending.status = 'pending'
             AND pending."createdAt" < now() - ($1 * interval '1 second')
           ORDER BY pending."createdAt" ASC
           LIMIT $2
           FOR UPDATE OF pending SKIP LOCKED"#,
        [timeout_seconds.into(), batch_size.into()],
    ))
    .all(&transaction)
    .await
    {
        Ok(expired) => expired,
        Err(error) => {
            let _ = transaction.rollback().await;
            state
                .metrics
                .record_worker_run("relay_config_timeout", "failure");
            tracing::error!(%error, worker = "relay_config_timeout", "failed to load stale relay configurations");
            return;
        }
    };
    if expired.is_empty() {
        let _ = transaction.commit().await;
        state
            .metrics
            .record_worker_run("relay_config_timeout", "success");
        return;
    }
    let ids: Vec<Uuid> = expired.iter().map(|pending| pending.id).collect();
    let now = chrono::Utc::now();
    if let Err(error) = pending_relay_config::Entity::update_many()
        .col_expr(
            pending_relay_config::Column::Status,
            Expr::value(pending_relay_config::PendingRelayConfigStatus::Failed),
        )
        .col_expr(pending_relay_config::Column::UpdatedAt, Expr::value(now))
        .filter(pending_relay_config::Column::Id.is_in(ids))
        .filter(
            pending_relay_config::Column::Status
                .eq(pending_relay_config::PendingRelayConfigStatus::Pending),
        )
        .exec(&transaction)
        .await
    {
        let _ = transaction.rollback().await;
        state
            .metrics
            .record_worker_run("relay_config_timeout", "failure");
        tracing::error!(%error, worker = "relay_config_timeout", "failed to expire relay configurations");
        return;
    }
    if let Err(error) = transaction.commit().await {
        state
            .metrics
            .record_worker_run("relay_config_timeout", "failure");
        tracing::error!(%error, worker = "relay_config_timeout", "failed to commit relay configuration expiry");
        return;
    }
    for pending in &expired {
        if let Err(error) = state.event_publisher.publish(relay_timeout_event(pending)) {
            tracing::warn!(%error, pending_id = %pending.id, "failed to publish relay timeout");
        }
    }
    state
        .metrics
        .record_worker_items("relay_config_timeout", expired.len() as u64);
    state
        .metrics
        .record_worker_run("relay_config_timeout", "success");
}
