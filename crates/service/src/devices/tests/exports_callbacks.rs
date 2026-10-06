use super::*;
#[tokio::test]
async fn device_csv_export_requires_admin_and_returns_attachment() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    config.device.build_upload_dir = directory.path().to_string_lossy().into_owned();
    let unauthenticated = api::router(Arc::new(
        AppState::without_dependencies(config.clone(), Metrics::new().unwrap())
            .with_device_repository(repository.clone()),
    ));
    let response = unauthenticated
        .oneshot(
            Request::get("/api/v1/device/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(repository);
    let authenticated = api::router(Arc::new(state));
    let response = authenticated
        .clone()
        .oneshot(
            Request::get("/api/v1/device/export?status=declined&search=Bench")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/csv");
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment; filename=\"devices-")
    );
    let body = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("deviceId,deviceName,deviceType"));
    assert!(body.contains("ethernet_interfaces"));
    assert!(body.contains("device-1,Bench 1,x5h"));
}

#[tokio::test]
async fn flashing_callback_is_public_and_reports_missing_devices() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    for (device_id, expected) in [
        ("device-1", StatusCode::OK),
        ("missing", StatusCode::NOT_FOUND),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/device/{device_id}/flashing"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        if expected == StatusCode::OK {
            assert_eq!(body["success"], true);
            assert_eq!(
                body["message"],
                "Device device-1 is now marked as upgrading"
            );
            let event = events.recv().await.unwrap();
            assert_eq!(event.event, "device_state_update");
            assert_eq!(event.payload["deviceId"], "device-1");
            assert_eq!(event.payload["state"], "busy");
            assert_eq!(event.payload["upgrading"], true);
            assert_eq!(event.payload["flashing"], false);
            assert!(event.payload["changedAt"].is_string());
        }
    }
}

#[tokio::test]
async fn test_completion_callback_sanitizes_ids_and_reports_errors() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }));
    let app = api::router(Arc::new(state));
    for (query, expected) in [
        ("testId=%20%22test-1%27%20", StatusCode::OK),
        ("testId=missing", StatusCode::NOT_FOUND),
        ("testId=%20%27%22%20", StatusCode::BAD_REQUEST),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/device/device-1/test-completed?{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body["success"], true);
            assert_eq!(
                body["message"],
                "Test completion handled for device device-1"
            );
        }
    }
}
