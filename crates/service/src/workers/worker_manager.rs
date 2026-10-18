use std::sync::Arc;

use farmcontroller_core::config::Module;
use tokio_util::sync::CancellationToken;

use crate::{
    state::AppState,
    workers::{
        artifact_scanner, device_action, device_action_watchdog, device_controller,
        device_heartbeat, fallback_flash, gen5_mapping, gen5_reboot_watchdog, relay_service,
        retention, system_metrics, test_preparation, types::WorkerManager,
    },
};

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
                retention::retention_worker(worker_state, worker_cancellation).await;
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
                    relay_service::relay_configuration_timeout_worker(
                        worker_state,
                        worker_cancellation,
                    )
                    .await;
                }));
                let worker_state = state.clone();
                let worker_cancellation = cancellation.child_token();
                handles.push(tokio::spawn(async move {
                    device_controller::controller_heartbeat_timeout_worker(
                        worker_state,
                        worker_cancellation,
                    )
                    .await;
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
