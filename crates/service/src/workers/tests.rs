//! Worker manager gating and timeout event contract tests.
//!
//! Covers disabled-module startup, relay-sync cancellation/status events, and
//! frontend payload shapes for relay/controller timeout alerts.

use std::sync::Arc;

use clap::Parser;
use farmcontroller_core::config::AppConfig;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    cli::Cli,
    devices::RelayControllerAction,
    observability::Metrics,
    socket::{
        helpers::{controller_timeout_events, relay_timeout_event},
        types::farm::EventHub,
    },
    state::AppState,
    workers::types::{
        ControllerStateTransition, ExpiredRelayConfiguration, RelaySyncService, WorkerManager,
    },
};

/// PURPOSE: Workers module off ⇒ manager starts zero tasks and shuts down cleanly.
/// USE: guards WorkerManager gating when workers module is disabled.
#[tokio::test]
async fn disabled_workers_start_no_tasks_and_shutdown_cleanly() {
    let config = AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap();
    let state = Arc::new(AppState::without_dependencies(
        config,
        Metrics::new().unwrap(),
    ));
    let manager = WorkerManager::start(state, CancellationToken::new());
    assert_eq!(manager.task_count(), 0);
    manager.shutdown().await;
}

/// PURPOSE: Relay sync task stops promptly on cancellation and publishes status events.
/// USE: regression for RelaySyncService worker lifecycle and failure events.
#[tokio::test]
async fn relay_sync_worker_is_bounded_and_cancellable() {
    let service = RelaySyncService::new(1);
    let cancellation = CancellationToken::new();
    let event_publisher = Arc::new(EventHub::default());
    let mut events = event_publisher.subscribe();
    let handle = service.start_worker(cancellation.clone(), event_publisher);
    service
        .enqueue(
            vec![RelayControllerAction::Remove {
                controller_address: "invalid address".to_owned(),
                device_mac: "AA:BB".to_owned(),
                relay_serial: "relay-1".to_owned(),
                channel_number: 1,
            }],
            None,
        )
        .unwrap();
    let progress = events.recv().await.unwrap();
    assert_eq!(progress.event, "relay_configuration_status");
    assert_eq!(progress.payload["status"], "in_progress");
    let failed = events.recv().await.unwrap();
    assert_eq!(failed.payload["status"], "failed");
    assert_eq!(failed.payload["total"], 1);
    assert_eq!(failed.payload["completed"], 0);
    assert_eq!(failed.payload["errors"].as_array().unwrap().len(), 1);
    assert!(
        failed.payload["message"]
            .as_str()
            .unwrap()
            .contains("Invalid controller address")
    );
    cancellation.cancel();
    handle.await.unwrap();
}

/// PURPOSE: Expired relay config event field names match the UI contract.
/// USE: guards relay_configuration_status timeout payload shaping.
#[test]
fn relay_timeout_event_preserves_frontend_contract() {
    let event = relay_timeout_event(&ExpiredRelayConfiguration {
        id: Uuid::nil(),
        device_mac: "aabbcc".to_owned(),
        channel_number: 2,
        relay_id: Some(Uuid::nil()),
        controller_id: Some("controller-1".to_owned()),
    });
    assert_eq!(event.event, "relay_configuration_status");
    assert_eq!(event.payload["status"], "failed");
    assert_eq!(event.payload["controllerId"], "controller-1");
    assert_eq!(event.payload["completed"], 0);
    assert_eq!(
        event.payload["message"],
        "Configuration timed out for device aabbcc on channel 2"
    );
}

/// PURPOSE: Controller timeout alert/event shapes match the UI contract.
/// USE: guards device_controller_changed + alert payloads on controller timeout.
#[test]
fn controller_timeout_events_preserve_frontend_contracts() {
    let [changed, alert] = controller_timeout_events(&ControllerStateTransition {
        controller_id: "controller-1".to_owned(),
        alert: serde_json::json!({"title": "Device Controller Not Reachable"}),
    });
    assert_eq!(changed.event, "device_controller_changed");
    assert_eq!(changed.payload["action"], "updated");
    assert_eq!(changed.payload["controllerId"], "controller-1");
    assert_eq!(alert.event, "alert");
    assert_eq!(alert.payload["subtype"], "device-alert");
    assert_eq!(
        alert.payload["message"]["title"],
        "Device Controller Not Reachable"
    );
}
