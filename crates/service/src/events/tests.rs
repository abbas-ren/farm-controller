#[test]
fn admin_terminal_mode_requires_the_frontend_client() {
    use super::{BrowserAuthorization, NativeSocketQuery, browser_authorization};

    let query = |client: Option<&str>, admin_terminal| NativeSocketQuery {
        device_controller_id: None,
        controller_id: None,
        device_id: None,
        test_id: None,
        client: client.map(str::to_owned),
        user_name: None,
        admin_terminal: Some(admin_terminal),
    };

    assert_eq!(
        browser_authorization(&query(Some("frontend"), true)),
        Ok(BrowserAuthorization::Admin)
    );
    assert_eq!(
        browser_authorization(&query(Some("frontend"), false)),
        Ok(BrowserAuthorization::Authenticated)
    );
    assert!(browser_authorization(&query(None, true)).is_err());
    assert!(browser_authorization(&query(Some("cli"), true)).is_err());
}

use axum::{body::Body, http::Request, routing::post};
use clap::Parser;
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::*;
use crate::{cli::Cli, config::AppConfig, observability::Metrics};

#[test]
fn controller_identity_normalization_matches_registration() {
    assert_eq!(normalize_controller_id("AA:BB:CC:DD:EE:FF"), "aabbccddeeff");
}

#[test]
fn edgecontroller_binary_payload_decodes() {
    let payload = serde_json::json!({
        "type": "heartbeat",
        "uid": "aabbccddeeff",
        "ip": "192.0.2.10",
        "timestamp": 1790899200,
        "cpuCurrent": "1.2 GHz",
        "cpuTotal": "1.8 GHz",
        "cpuUsagePercent": 12.5,
        "memoryUsed": "1.0 GB",
        "memoryTotal": "4.0 GB",
        "memoryUsagePercent": 25.0,
        "networkUpload": "0.1 Mbps",
        "networkDownload": "1.0 Mbps",
        "diskUsed": "8.0 GB",
        "diskTotal": "32.0 GB",
        "diskUsagePercent": 25.0
    });
    let heartbeat: ControllerHeartbeat =
        serde_json::from_slice(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert_eq!(heartbeat.message_type, "heartbeat");
    assert_eq!(heartbeat.uid, "aabbccddeeff");
    let event = controller_heartbeat_event(heartbeat.uid.clone(), &heartbeat);
    assert_eq!(event.event, "device_controller:ping");
    assert_eq!(
        event.room.as_deref(),
        Some("device-controller:aabbccddeeff")
    );
    assert_eq!(event.payload["controllerId"], "aabbccddeeff");
    assert_eq!(event.payload["timestamp"], "2026-10-02T00:00:00.000Z");
    assert_eq!(event.payload["metrics"]["cpu"]["usagePercent"], 12.5);
    assert_eq!(event.payload["metrics"]["memory"]["used"], "1.0 GB");
    assert_eq!(event.payload["metrics"]["network"]["download"], "1.0 Mbps");
    assert_eq!(event.payload["metrics"]["disk"]["total"], "32.0 GB");
}

#[test]
fn device_heartbeat_events_follow_committed_frontend_contract() {
    let events = device_heartbeat_events(
        "device-1",
        DeviceHeartbeatResult {
            state: "free".to_owned(),
            state_changed: true,
            interface_changes: vec![serde_json::json!({
                "type": "ethernet",
                "interfaceId": "eth0",
                "status": "up",
                "previousStatus": "down",
            })],
            heartbeat_data: serde_json::json!({"ethernet": {"eth0": {"status": "up"}}}),
            alerts: vec![serde_json::json!({"title": "Device comes as Available"})],
            post_update_events: Vec::new(),
            deferred_cancellations: Vec::new(),
        },
        "2026-10-02T00:00:00.000Z",
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.event.as_str())
            .collect::<Vec<_>>(),
        vec![
            "device_state_update",
            "device:interface-update",
            "alert",
            "device:ping"
        ]
    );
    assert_eq!(events[0].payload["state"], "free");
    assert_eq!(events[1].room.as_deref(), Some("device:device-1"));
    assert_eq!(events[1].payload["changes"][0]["interfaceId"], "eth0");
    assert_eq!(events[2].payload["subtype"], "device-alert");
    assert_eq!(events[3].room.as_deref(), Some("device:device-1"));
    assert_eq!(events[3].payload["deviceId"], "device-1");
}

#[test]
fn socket_io_room_and_build_fanout_match_frontend_contracts() {
    assert_eq!(
        ROOM_REGISTRATIONS,
        [
            ("join:build", "leave:build", "build"),
            ("join:test", "leave:test", "test"),
            ("join:dashboard", "leave:dashboard", "dashboard"),
            ("join:device", "leave:device", "device"),
            (
                "join:device-controller",
                "leave:device-controller",
                "device-controller"
            ),
        ]
    );
    let events = build_performance_events("build-1");
    assert!(events.iter().all(|event| {
        event.event == "build_performance_update" && event.payload["buildId"] == "build-1"
    }));
    assert_eq!(events[0].room.as_deref(), Some("dashboard:admin"));
    assert_eq!(events[1].room.as_deref(), Some("dashboard:user"));
    assert_eq!(events[2].room.as_deref(), Some("build:build-1"));
}

#[tokio::test]
async fn user_message_dispatch_preserves_public_event_contract() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state = Arc::new(AppState::without_dependencies(
        AppConfig::load(&cli).unwrap(),
        Metrics::new().unwrap(),
    ));
    let mut events = state.event_publisher.subscribe();
    let app = Router::new()
        .route("/api/v1/device/ws/send/message", post(send_user_message))
        .with_state(state);
    let response = app
        .oneshot(
            Request::post("/api/v1/device/ws/send/message")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"data":{"method":"refresh"}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body, serde_json::json!({"ok": true}));
    let event = events.recv().await.unwrap();
    assert_eq!(event.event, "user");
    assert_eq!(event.payload["message"]["method"], "refresh");
    assert!(event.payload["timestamp"].is_string());
}
