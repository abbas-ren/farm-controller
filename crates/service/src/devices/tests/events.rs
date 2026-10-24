//! Frontend Socket.IO event payload contract tests for device flows.
//!
//! Asserts power, execution, cancellation, report-upload, and relay confirmation events
//! match the committed frontend room/event schema.

use super::*;

/// PURPOSE: Assert device power event preserves frontend contract.
/// USE: regression guard for legacy/API wire contracts covered by `events`.
#[test]
fn device_power_event_preserves_frontend_contract() {
    let event = device_power_event("device-1", "off");
    assert_eq!(event.event, "device_power_update");
    assert_eq!(event.payload["deviceId"], "device-1");
    assert_eq!(event.payload["power"], "off");
    assert!(event.payload["changedAt"].is_string());
}

/// PURPOSE: Assert new execution event preserves frontend contract.
/// USE: regression guard for legacy/API wire contracts covered by `events`.
#[test]
fn new_execution_event_preserves_frontend_contract() {
    let event = test_execution_handlers::new_execution_event("test-1", "user-1");
    assert_eq!(event.event, "testExecution");
    assert_eq!(event.payload["testId"], "test-1");
    assert_eq!(event.payload["type"], "new");
    assert_eq!(event.payload["data"]["status"], "notExecuted");
    assert_eq!(
        event.payload["data"]["message"],
        "Test execution test-1 created"
    );
    assert!(event.payload["data"]["timestamp"].is_string());
}

/// PURPOSE: Assert cancellation events preserve user and build contracts.
/// USE: regression guard for legacy/API wire contracts covered by `events`.
#[test]
fn cancellation_events_preserve_user_and_build_contracts() {
    let [user_event, build_event] = test_execution_handlers::cancellation_events(
        "test-1",
        "build-1",
        "PREPARE_ARTIFACTS",
        "user-1",
    );
    assert_eq!(user_event.event, "testExecution");
    assert_eq!(user_event.payload["type"], "cancelRequested");
    assert_eq!(user_event.payload["data"]["cancelRequested"], true);
    assert_eq!(user_event.payload["data"]["phase"], "PREPARE_ARTIFACTS");
    assert_eq!(build_event.event, "testExecutionUpdate");
    assert_eq!(build_event.payload["testId"], "test-1");
    assert_eq!(build_event.payload["buildId"], "build-1");
    assert_eq!(build_event.payload["cancelRequested"], true);
}

/// PURPOSE: Assert report upload events preserve frontend status and room contracts.
/// USE: regression guard for legacy/API wire contracts covered by `events`.
#[test]
fn report_upload_events_preserve_frontend_status_and_room_contracts() {
    let report_id = Uuid::new_v4();
    let events = test_execution_handlers::report_update_events(
        report_id,
        "test-1",
        "user-1",
        "failed",
        Some("Confluence upload failed: rejected"),
        Some("rejected"),
    );
    assert_eq!(events[0].event, "execution_report_update");
    assert_eq!(events[0].payload["reportId"], report_id.to_string());
    assert_eq!(events[0].payload["testExecutionId"], "test-1");
    assert_eq!(events[0].payload["status"], "failed");
    assert_eq!(events[0].payload["createdBy"], "user-1");
    assert_eq!(
        events[0].payload["error"],
        "Confluence upload failed: rejected"
    );
    assert_eq!(events[0].payload["uploadError"], "rejected");
}

/// PURPOSE: Assert relay confirmation events follow committed context.
/// USE: regression guard for legacy/API wire contracts covered by `events`.
#[test]
fn relay_confirmation_events_follow_committed_context() {
    let response = RelayConfirmationResponse {
        confirmed: true,
        message: "confirmed".to_owned(),
        controller_id: Some("controller-1".to_owned()),
        relay_id: Some(Uuid::nil()),
        device_id: Some("device-1".to_owned()),
        configuration_complete: true,
    };
    let events = relay_handlers::confirmation_events(&response);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].event, "device_power_update");
    assert_eq!(events[0].payload["deviceId"], "device-1");
    assert_eq!(events[1].event, "relay_configuration_status");
    assert_eq!(events[1].payload["status"], "completed");
    assert_eq!(events[1].payload["completed"], 1);
    assert_eq!(events[2].event, "device_controller_changed");
    assert_eq!(events[2].payload["controllerId"], "controller-1");
}
