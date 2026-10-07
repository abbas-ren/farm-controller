#[cfg(test)]
pub mod tests;

use std::{sync::Arc, time::Duration};

use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    config::Module,
    devices::{EDGE_CONTROLLER_PORT, RELAY_CONTROLLER_TIMEOUT, RelayControllerAction},
    events::{EventHub, ServerEvent},
    state::AppState,
};

mod artifact_preparation;
mod artifact_scanner;
mod build_preparation;
mod device_action;
mod device_action_watchdog;
mod device_heartbeat;
mod fallback_flash;
mod gen5_mapping;
mod gen5_reboot_watchdog;
mod ipl_distribution;
mod system_metrics;
mod test_preconditions;
mod test_preparation;

pub(crate) use build_preparation::prepare_uploaded_build;
pub(crate) use ipl_distribution::{
    remove_build as remove_build_ipl, sync_controller as sync_controller_ipl,
};

struct RelaySyncJob {
    actions: Vec<RelayControllerAction>,
    relay_id: Option<uuid::Uuid>,
}

#[derive(Debug, sqlx::FromRow)]
struct ExpiredRelayConfiguration {
    id: Uuid,
    device_mac: String,
    channel_number: i32,
    relay_id: Option<Uuid>,
    controller_id: Option<String>,
}

struct ControllerStateTransition {
    controller_id: String,
    alert: serde_json::Value,
}

#[derive(Clone)]
pub(crate) struct RelaySyncService {
    sender: mpsc::Sender<RelaySyncJob>,
    receiver: Arc<Mutex<Option<mpsc::Receiver<RelaySyncJob>>>>,
}

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

pub struct WorkerManager {
    cancellation: CancellationToken,
    handles: Vec<JoinHandle<()>>,
}

impl WorkerManager {
    pub fn start(state: Arc<AppState>, cancellation: CancellationToken) -> Self {
        let mut handles = Vec::new();
        if state.config.module_enabled(Module::Events) {
            let event_publisher = state.event_publisher.clone();
            let worker_cancellation = cancellation.child_token();
            handles.push(tokio::spawn(async move {
                system_metrics::worker(event_publisher, worker_cancellation).await;
            }));
        }
        if state.config.module_enabled(Module::Workers) && state.database.is_some() {
            let worker_state = state.clone();
            let worker_cancellation = cancellation.child_token();
            handles.push(tokio::spawn(async move {
                retention_worker(worker_state, worker_cancellation).await;
            }));
        }
        if state.config.module_enabled(Module::Workers)
            && state.config.module_enabled(Module::Device)
        {
            handles.push(
                state
                    .relay_sync
                    .start_worker(cancellation.child_token(), state.event_publisher.clone()),
            );
            if state.database.is_some() {
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    relay_configuration_timeout_worker(worker_state, worker_cancellation).await;
                }));
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    controller_heartbeat_timeout_worker(worker_state, worker_cancellation).await;
                }));
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    device_heartbeat::worker(worker_state, worker_cancellation).await;
                }));
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    device_action_watchdog::worker(worker_state, worker_cancellation).await;
                }));
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    test_preparation::worker(worker_state, worker_cancellation).await;
                }));
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    artifact_scanner::worker(worker_state, worker_cancellation).await;
                }));
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    device_action::worker(worker_state, worker_cancellation).await;
                }));
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    fallback_flash::worker(worker_state, worker_cancellation).await;
                }));
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    gen5_mapping::worker(worker_state, worker_cancellation).await;
                }));
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    gen5_reboot_watchdog::worker(worker_state, worker_cancellation).await;
                }));
            }
        }
        if let Some(reports) = &state.reports
            && let Some(handle) = reports.start_worker(cancellation.child_token())
        {
            handles.push(handle);
        }
        Self {
            cancellation,
            handles,
        }
    }

    pub async fn shutdown(self) {
        self.cancellation.cancel();
        for handle in self.handles {
            if let Err(error) = handle.await {
                tracing::error!(%error, "worker task failed while shutting down");
            }
        }
    }

    pub fn task_count(&self) -> usize {
        self.handles.len()
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
                Duration::from_secs(1),
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
    let event = ServerEvent {
        event: "relay_configuration_status".to_owned(),
        payload: serde_json::json!({
            "relayId": relay_id,
            "status": status,
            "total": total,
            "completed": completed,
            "errors": errors,
            "message": message,
        }),
        room: None,
    };
    if let Err(error) = event_publisher.publish(event) {
        tracing::warn!(%error, status, "failed to publish relay synchronization status");
    }
}

fn device_generation_number(value: &str) -> Option<u32> {
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

async fn relay_configuration_timeout_worker(state: Arc<AppState>, cancellation: CancellationToken) {
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
    let mut transaction = match database.pool().begin().await {
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
    let expired = match sqlx::query_as::<_, ExpiredRelayConfiguration>(
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
    )
    .bind(timeout_seconds)
    .bind(batch_size)
    .fetch_all(&mut *transaction)
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
    if let Err(error) = sqlx::query(
        r#"UPDATE pending_relay_configs
           SET status = 'failed', "updatedAt" = now()
           WHERE id = ANY($1) AND status = 'pending'"#,
    )
    .bind(&ids)
    .execute(&mut *transaction)
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

fn relay_timeout_event(pending: &ExpiredRelayConfiguration) -> ServerEvent {
    let message = format!(
        "Configuration timed out for device {} on channel {}",
        pending.device_mac, pending.channel_number
    );
    ServerEvent {
        event: "relay_configuration_status".to_owned(),
        payload: serde_json::json!({
            "controllerId": pending.controller_id.as_deref(),
            "relayId": pending.relay_id,
            "status": "failed",
            "total": 1,
            "completed": 0,
            "errors": [&message],
            "message": message,
        }),
        room: None,
    }
}

async fn controller_heartbeat_timeout_worker(
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

fn controller_timeout_events(transition: &ControllerStateTransition) -> [ServerEvent; 2] {
    [
        ServerEvent {
            event: "device_controller_changed".to_owned(),
            payload: serde_json::json!({
                "action": "updated",
                "controllerId": transition.controller_id,
            }),
            room: None,
        },
        ServerEvent::alert("device-alert", transition.alert.clone()),
    ]
}

async fn retention_worker(state: Arc<AppState>, cancellation: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.retention_interval_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_retention_cycle(&state).await,
        }
    }
    tracing::info!(worker = "retention", "worker stopped");
}

async fn run_retention_cycle(state: &AppState) {
    let Some(database) = &state.database else {
        return;
    };
    let policies = match database.enabled_retention_policies().await {
        Ok(policies) => policies,
        Err(error) => {
            state.metrics.record_worker_run("retention", "failure");
            tracing::error!(%error, worker = "retention", "failed to load retention policies");
            return;
        }
    };
    let mut cycle_failed = false;
    for policy in policies {
        for _ in 0..state.config.workers.retention_max_batches_per_policy {
            match database.clean_retention_batch(&policy).await {
                Ok(deleted) => {
                    state.metrics.record_worker_items("retention", deleted);
                    tracing::info!(
                        worker = "retention",
                        table = policy.table_name,
                        deleted,
                        "retention batch completed"
                    );
                    if deleted < policy.batch_size as u64 {
                        break;
                    }
                }
                Err(error) => {
                    cycle_failed = true;
                    tracing::error!(
                        %error,
                        worker = "retention",
                        table = policy.table_name,
                        "retention policy failed"
                    );
                    break;
                }
            }
        }
    }
    state.metrics.record_worker_run(
        "retention",
        if cycle_failed { "failure" } else { "success" },
    );
}
