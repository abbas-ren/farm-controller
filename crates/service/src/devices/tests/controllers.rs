use super::*;
#[test]
fn edgecontroller_state_map_becomes_channel_inventory() {
    let relay = RelayRegistration {
        serial_number: "relay-1".to_owned(),
        state: BTreeMap::from([("channel_1".to_owned(), 0), ("channel_8".to_owned(), 1)]),
        channels: Vec::new(),
    };
    let channels = validation::relay_channels(&relay).unwrap();
    assert_eq!(channels.len(), 2);
    assert_eq!(channels[0].channel_number, 1);
    assert_eq!(channels[1].channel_number, 8);
}

#[test]
fn controller_identity_normalizes_common_mac_formats() {
    assert_eq!(
        validation::normalized_controller_id("AA:bb:CC:dd:EE:ff").unwrap(),
        "aabbccddeeff"
    );
    assert_eq!(
        validation::normalized_controller_id("AA-BB-CC-DD-EE-FF").unwrap(),
        "aabbccddeeff"
    );
}

#[tokio::test]
async fn edgecontroller_registration_is_create_then_idempotent_update() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let payload = serde_json::json!({
        "macAddress": "AA:BB:CC:DD:EE:FF",
        "ipAddress": "192.0.2.10",
        "deviceFamily": "Gen5",
        "relays": [{
            "serialNumber": "relay-1",
            "state": {"channel_0": 0, "channel_1": 1}
        }],
        "uid": "aabbccddeeff"
    });
    for (path, expected) in [
        ("/api/v1/device/controller/", StatusCode::CREATED),
        ("/api/v1/device/controller", StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["success"], true);
        assert_eq!(body["data"]["deviceControllerId"], "aabbccddeeff");
        let event = events.recv().await.unwrap();
        assert_eq!(event.event, "device_controller_changed");
        assert_eq!(
            event.payload["action"],
            if expected == StatusCode::CREATED {
                "added"
            } else {
                "updated"
            }
        );
        assert_eq!(event.payload["controllerId"], "aabbccddeeff");
        assert!(event.room.is_none());
        if expected == StatusCode::CREATED {
            let alert = events.recv().await.unwrap();
            assert_eq!(alert.event, "alert");
            assert_eq!(alert.payload["subtype"], "device-controller-addition");
        }
    }
}

#[tokio::test]
async fn registration_request_publishes_pending_user_frontend_event() {
    let mut config = AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap();
    config.modules.enabled.insert(crate::config::Module::Auth);
    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    );
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let response = app
        .oneshot(
            Request::post("/api/v1/auth/register/request")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "username": "pending.user",
                        "email": "pending@example.com",
                        "firstName": "Pending",
                        "lastName": "User"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let event = events.recv().await.unwrap();
    assert_eq!(event.event, "user");
    assert!(event.room.is_none());
    assert_eq!(event.payload["message"]["id"], "pending-user-1");
    assert_eq!(event.payload["message"]["username"], "pending.user");
    assert_eq!(event.payload["message"]["email"], "pending@example.com");
    assert_eq!(
        event.payload["message"]["requiredActions"][0],
        "UPDATE_PASSWORD"
    );
    assert!(event.payload["timestamp"].is_string());
}

#[tokio::test]
async fn device_registration_is_public_and_preserves_wire_contract() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let response = app
        .oneshot(
            Request::post("/api/v1/device/")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "macAddress": "00:1A:2B:3C:4D:5E",
                        "ipAddress": "192.0.2.20",
                        "deviceName": "Bench device",
                        "interfaces": {"ethernet": ["eth0"]}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["deviceId"], "001a2b3c4d5e");
    assert_eq!(body["data"]["heartbeatTimer"], 5);
    assert_eq!(body["data"]["interfaces"][0]["type"], "ethernet");
    for subtype in ["device-addition", "device-approval"] {
        let event = events.recv().await.unwrap();
        assert_eq!(event.event, "alert");
        assert_eq!(event.payload["type"], "alert");
        assert_eq!(event.payload["subtype"], subtype);
        assert_eq!(event.payload["message"]["deviceId"], "001a2b3c4d5e");
        assert!(event.payload["timestamp"].is_string());
    }
}

#[tokio::test]
async fn device_registration_maps_invalid_and_missing_fields_to_bad_request() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }));
    let app = api::router(Arc::new(state));

    for payload in [
        serde_json::json!({
            "macAddress": "invalid",
            "ipAddress": "192.0.2.20",
            "deviceName": "Bench device"
        }),
        serde_json::json!({
            "macAddress": "00:1A:2B:3C:4D:5E",
            "deviceName": "Bench device"
        }),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/device/")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
