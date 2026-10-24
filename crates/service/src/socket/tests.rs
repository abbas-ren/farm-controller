//! Socket helper and route contract tests.
//!
//! Guards browser auth, EdgeController heartbeat decoding, device heartbeat fan-out,
//! and public user-message dispatch against the committed Socket.IO / native WS contracts.

use std::sync::Arc;

use axum::{Router, body::Body, http::Request, routing::post};
use clap::Parser;
use http_body_util::BodyExt;
use reqwest::StatusCode;
use tower::ServiceExt;

use crate::{
    cli::Cli,
    config::AppConfig,
    devices::{ControllerHeartbeat, DeviceHeartbeatResult},
    observability::Metrics,
    socket::{
        constants::WS_EVENT,
        helpers::{
            browser_auth_headers, browser_authorization, controller_heartbeat_payload,
            device_heartbeat_events, normalize_controller_id,
        },
        routes::user::send_user_message,
        types::farm::ServerEvent,
    },
    state::AppState,
};

/// PURPOSE: Admin terminal mode requires the frontend client query flag.
/// USE: guards native socket BrowserAuthorization admin gating.
#[test]
fn admin_terminal_mode_requires_the_frontend_client() {
    use super::types::farm::{BrowserAuthorization, NativeSocketQuery};

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

/// PURPOSE: Browser websocket subprotocol supplies bearer authentication.
/// USE: regression for farm-browser-v1 + bearer token subprotocol parsing.
#[test]
fn browser_websocket_subprotocol_supplies_bearer_authentication() {
    use axum::http::{HeaderMap, HeaderValue, header};

    let mut headers = HeaderMap::new();
    headers.insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("farm-browser-v1, bearer.header.payload.signature"),
    );
    let authorized = browser_auth_headers(&headers);
    assert_eq!(
        authorized
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer header.payload.signature")
    );
}

/// PURPOSE: Controller identity normalization matches registration.
/// USE: keeps socket heartbeat IDs aligned with device registration MAC normalization.
#[test]
fn controller_identity_normalization_matches_registration() {
    assert_eq!(normalize_controller_id("AA:BB:CC:DD:EE:FF"), "aabbccddeeff");
}

/// PURPOSE: EdgeController binary heartbeat payload decodes to frontend metrics event.
/// USE: regression for controller ping Socket.IO payload shaping.
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
    let event = ServerEvent::frontend(controller_heartbeat_payload(
        heartbeat.uid.clone(),
        &heartbeat,
    ));
    assert_eq!(event.event, WS_EVENT.DEVICE_CONTROLLER_PING);

    assert_eq!(event.payload["controllerId"], "aabbccddeeff");
    assert_eq!(event.payload["timestamp"], "2026-10-02T00:00:00.000Z");
    assert_eq!(event.payload["metrics"]["cpu"]["usagePercent"], 12.5);
    assert_eq!(event.payload["metrics"]["memory"]["used"], "1.0 GB");
    assert_eq!(event.payload["metrics"]["network"]["download"], "1.0 Mbps");
    assert_eq!(event.payload["metrics"]["disk"]["total"], "32.0 GB");
}

/// PURPOSE: Device heartbeat events follow the committed frontend contract.
/// USE: guards state/interface/alert/ping event fan-out from heartbeat results.
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
            deferred_test_logs: Vec::new(),
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
    assert_eq!(events[1].payload["changes"][0]["interfaceId"], "eth0");
    assert_eq!(events[2].payload["subtype"], "device-alert");
    assert_eq!(events[3].payload["deviceId"], "device-1");
}

/// PURPOSE: User message dispatch preserves the public event contract.
/// USE: regression for `/ws/send/message` → `user` Socket.IO publish.
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
