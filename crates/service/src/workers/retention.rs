use crate::state::AppState;
use std::sync::Arc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

pub(crate) async fn retention_worker(state: Arc<AppState>, cancellation: CancellationToken) {
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
