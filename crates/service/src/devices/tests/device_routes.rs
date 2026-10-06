use super::*;
#[tokio::test]
async fn edgecontroller_callbacks_preserve_wire_contracts() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(repository.clone());
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));

    let mapping = serde_json::json!({
        "mac": "AA:BB:CC:DD:EE:FF",
        "ip": "192.0.2.10",
        "status": "success",
        "tty_entry": "{\"uart\":\"/dev/ttyUSB0\",\"power\":\"/dev/ttyUSB1\"}"
    });
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/mapping-gen5")
                .header("content-type", "application/json")
                .body(Body::from(mapping.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    for path in [
        "/api/v1/device/flash-confirm?status=success",
        "/api/v1/device/flash-confirm-gen4?status=failure",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    assert_eq!(
        *repository.callbacks.lock().unwrap(),
        [
            "mapping:192.0.2.10:AA:BB:CC:DD:EE:FF:Success:/dev/ttyUSB0",
            "flash:x5h:Success",
            "flash:v4h:Failure",
        ]
    );
    let cancellation = events.recv().await.unwrap();
    assert_eq!(cancellation.event, "test_execution_update");
    assert_eq!(cancellation.payload["testId"], "test-1");
    assert_eq!(
        cancellation.payload["update"]["data"]["status"],
        "cancelled"
    );
    assert_eq!(cancellation.room.as_deref(), Some("user:user-1"));
    let device = events.recv().await.unwrap();
    assert_eq!(device.event, "device_state_update");
    assert_eq!(device.payload["deviceId"], "device-1");
    assert_eq!(device.payload["state"], "free");
}

#[tokio::test]
async fn frontend_family_and_type_reads_require_auth_and_return_bare_arrays() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let config = AppConfig::load(&cli).unwrap();
    let unauthenticated = api::router(Arc::new(
        AppState::without_dependencies(config.clone(), Metrics::new().unwrap())
            .with_device_repository(repository.clone()),
    ));
    let response = unauthenticated
        .oneshot(
            Request::get("/api/v1/device/families")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let authenticated = api::router(Arc::new(
        AppState::with_identity_provider(
            config,
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(repository),
    ));
    for (path, expected) in [
        (
            "/api/v1/device/families",
            serde_json::json!(["Gen4", "Gen5"]),
        ),
        (
            "/api/v1/device/deviceTypes?deviceFamily=Gen5",
            serde_json::json!(["x5h"]),
        ),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::get(path)
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body, expected);
    }

    for (path, expected_data) in [
        (
            "/api/v1/device/device-1",
            serde_json::json!({
                "deviceId": "device-1",
                "deviceFamily": "Gen5",
                "interfaces": [],
                "testExecutions": []
            }),
        ),
        (
            "/api/v1/device/device-1/heartbeat",
            serde_json::json!({
                "timestamp": "2026-10-02T00:00:00Z",
                "data": {"cpuUsagePercent": 12.5},
                "timeout": 30
            }),
        ),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::get(path)
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["success"], true);
        assert_eq!(body["data"], expected_data);
    }

    let response = authenticated
        .oneshot(
            Request::get("/api/v1/device/missing")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ))
    .oneshot(
        Request::get("/api/v1/device/topology")
            .header("authorization", "Bearer access")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"][0]["controllerId"], "aabbccddeeff");
}

#[tokio::test]
async fn relay_configuration_preserves_admin_and_callback_contracts() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state = AppState::with_identity_provider(
        AppConfig::load(&cli).unwrap(),
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(repository);
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let relay_id = Uuid::new_v4();
    let channel_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "relayId": relay_id,
        "channelId": channel_id,
        "deviceId": "device-1",
        "gpio": "17",
        "gpioDefaultLevel": "LOW",
        "relayDefaultLevel": "LOW"
    });

    let unauthorized = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/relay/configure")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let accepted = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/relay/configure")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let body: serde_json::Value =
        serde_json::from_slice(&accepted.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"]["id"], channel_id.to_string());

    let confirmation = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/relay/config/confirmation")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"mac":"AA:BB","status":"success"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(confirmation.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&confirmation.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(body["confirmed"], true);
    let power_event = events.recv().await.unwrap();
    assert_eq!(power_event.event, "device_power_update");
    assert_eq!(power_event.payload["deviceId"], "device-1");
    let status_event = events.recv().await.unwrap();
    assert_eq!(status_event.event, "relay_configuration_status");
    assert_eq!(status_event.payload["status"], "completed");
    assert_eq!(status_event.payload["completed"], 1);
    let controller_event = events.recv().await.unwrap();
    assert_eq!(controller_event.event, "device_controller_changed");
    assert_eq!(controller_event.payload["controllerId"], "controller-1");

    let invalid = app
        .oneshot(
            Request::post("/api/v1/device/relay/config/confirmation")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"mac":"AA:BB","status":"unknown"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn device_power_toggle_preserves_frontend_http_and_event_contracts() {
    let _controller_guard = edge_controller_test_lock().lock().await;
    let controller_requests = Arc::new(Mutex::new(Vec::new()));
    let captured_requests = Arc::clone(&controller_requests);
    let controller = axum::Router::new().route(
        "/relay",
        axum::routing::post(move |Json(payload): Json<serde_json::Value>| {
            let captured_requests = Arc::clone(&captured_requests);
            async move {
                captured_requests.lock().unwrap().push(payload);
                StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", EDGE_CONTROLLER_PORT))
        .await
        .unwrap();
    let controller_task = tokio::spawn(async move {
        axum::serve(listener, controller).await.unwrap();
    });

    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let state = AppState::with_identity_provider(
        AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap(),
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(repository.clone());
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));

    let response = app
        .oneshot(
            Request::put("/api/v1/device/relay/toggle/power-device")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["deviceId"], "power-device");
    assert_eq!(body["channelNumber"], 2);
    assert_eq!(body["state"], "off");
    assert_eq!(
        controller_requests.lock().unwrap().as_slice(),
        &[serde_json::json!({"serial": "relay-1", "state": "off", "channel": 2})]
    );
    assert_eq!(
        repository.callbacks.lock().unwrap().as_slice(),
        &["power:power-device:off"]
    );
    let event = events.recv().await.unwrap();
    assert_eq!(event.event, "device_power_update");
    assert_eq!(event.payload["deviceId"], "power-device");
    assert_eq!(event.payload["power"], "off");
    assert!(event.payload["changedAt"].is_string());
    assert!(event.room.is_none());

    controller_task.abort();
    let _ = controller_task.await;
}

#[tokio::test]
async fn heartbeat_timeout_preserves_frontend_and_device_callback_contracts() {
    let _controller_guard = edge_controller_test_lock().lock().await;
    let controller_requests = Arc::new(Mutex::new(Vec::new()));
    let captured_requests = Arc::clone(&controller_requests);
    let controller = axum::Router::new().route(
        "/configure/heartbeat",
        axum::routing::post(move |Json(payload): Json<serde_json::Value>| {
            let captured_requests = Arc::clone(&captured_requests);
            async move {
                captured_requests.lock().unwrap().push(payload);
                StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", EDGE_CONTROLLER_PORT))
        .await
        .unwrap();
    let controller_task = tokio::spawn(async move {
        axum::serve(listener, controller).await.unwrap();
    });

    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(repository.clone()),
    ));
    let response = app
        .oneshot(
            Request::put("/api/v1/device/heartbeat/timeout")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"deviceId": "heartbeat-device", "value": 15}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["message"], "Heartbeat timeout saved successfully.");
    assert_eq!(
        controller_requests.lock().unwrap().as_slice(),
        &[serde_json::json!({"timeout": 15})]
    );
    assert_eq!(
        repository.callbacks.lock().unwrap().as_slice(),
        &["heartbeat:heartbeat-device:15"]
    );

    controller_task.abort();
    let _ = controller_task.await;
}

#[tokio::test]
async fn available_relay_devices_preserves_frontend_picker_contract() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(repository.clone()),
    ));
    let relay_id = Uuid::new_v4();

    let unauthorized = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/relay/devices/available")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::get(format!(
                "/api/v1/device/relay/devices/available?page=2&limit=5&relayId={relay_id}"
            ))
            .header("authorization", "Bearer access")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["devices"][0]["deviceId"], "device-1");
    assert_eq!(body["devices"][0]["deviceType"], "Racer");
    assert_eq!(body["devices"][0]["macAddress"], "00:11:22:33:44:55");
    assert_eq!(body["pagination"]["page"], 2);
    assert_eq!(body["pagination"]["limit"], 5);
    assert_eq!(body["pagination"]["totalCount"], 1);
    assert_eq!(body["pagination"]["totalPages"], 1);
    assert_eq!(
        repository.callbacks.lock().unwrap().as_slice(),
        &[format!("available:2:5:{relay_id}")]
    );
}

#[tokio::test]
async fn retained_relay_routes_preserve_legacy_contracts() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&cli).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ));
    let relay_id = Uuid::new_v4();
    let channel_id = Uuid::new_v4();

    let unauthorized = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/relay")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let configured = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/relay/config")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "id": channel_id,
                        "relayId": relay_id,
                        "deviceId": "device-1",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(configured.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&configured.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(body["data"]["id"], channel_id.to_string());
    assert_eq!(body["message"], "Device configured with relay successfully");

    for (path, payload) in [
        (
            "/api/v1/device/relay/config/fresh",
            serde_json::json!({"relayId": relay_id}),
        ),
        (
            "/api/v1/device/relay/config/remap",
            serde_json::json!({"relayId": relay_id}),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("authorization", "Bearer access")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let conflicts = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/device/relay/check-conflicts?relayId={relay_id}&channelNumber=1&deviceId=device-1"
            ))
            .header("authorization", "Bearer access")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflicts.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&conflicts.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["conflict"]["deviceConflict"], true);
    assert_eq!(body["conflict"]["channelConflict"], true);

    for path in [
        "/api/v1/device/relay?controllerId=controller-1".to_owned(),
        format!("/api/v1/device/relay/channel?relayId={relay_id}"),
        format!("/api/v1/device/relay/channel/{channel_id}"),
        format!("/api/v1/device/relay/{relay_id}"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(path)
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    for path in [
        format!("/api/v1/device/relay/channel/{channel_id}"),
        format!("/api/v1/device/relay/{relay_id}"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::delete(path)
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}
