use super::*;
fn log_test_app() -> axum::Router {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&cli).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ))
}

fn execution_creation_test_app(with_catalog: bool) -> axum::Router {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state = AppState::with_identity_provider(
        AppConfig::load(&cli).unwrap(),
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    }));
    let state = if with_catalog {
        state.with_test_catalog(Arc::new(FakeTestCatalog))
    } else {
        state
    };
    api::router(Arc::new(state))
}

#[tokio::test]
async fn execution_creation_preserves_legacy_payload_and_auth_contract() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state = AppState::with_identity_provider(
        AppConfig::load(&cli).unwrap(),
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let payload = serde_json::json!({
        "name": "Smoke execution",
        "deviceFamily": "R-Car",
        "deviceType": "x5h",
        "buildId": "build-1",
        "testPlanId": 7,
        "testPlanName": "Smoke",
        "testCases": {
            "8": [{
                "testCaseId": 9,
                "suiteId": 8,
                "scriptFile": "run.sh",
                "suiteName": "Core",
                "title": "Boot",
                "planId": 7
            }]
        }
    });
    let unauthorized = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/test/execution")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/test/execution")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["testId"], "created-test-1");
    assert_eq!(body["status"], "not_executed");
    assert_eq!(body["executionPhase"], "PREPARE_ARTIFACTS");
    assert_eq!(body["testCases"][0]["testCaseId"], 9);
    assert_eq!(body["createdBy"], "system");
    let event = events.recv().await.unwrap();
    assert_eq!(event.event, "test_execution");
    assert_eq!(event.payload["testId"], body["testId"]);
    assert_eq!(event.payload["type"], "new");
    assert_eq!(event.payload["data"]["status"], body["status"]);
    assert_eq!(event.room.as_deref(), Some("user:system"));

    let invalid = app
        .oneshot(
            Request::post("/api/v1/device/test/execution")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "deviceFamily": "R-Car",
                        "deviceType": "x5h",
                        "buildId": "build-1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn execution_creation_expands_selection_through_test_catalog() {
    let response = execution_creation_test_app(true)
        .oneshot(
            Request::post("/api/v1/device/test/execution")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "deviceFamily": "R-Car",
                        "deviceType": "x5h",
                        "buildId": "build-1",
                        "selection": {
                            "mode": "PARTIAL",
                            "planId": "P7",
                            "planName": "Plan",
                            "suites": [{"suiteId": "S8", "selectAll": true}]
                        }
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
    assert_eq!(body["testPlanId"], 7);
    assert_eq!(body["testSuits"], serde_json::json!([8]));
    assert_eq!(body["testCases"][0]["testCaseId"], 9);
    assert_eq!(body["testCases"][0]["suiteName"], "Core");
    assert_eq!(body["selectionInput"]["mode"], "PARTIAL");
}

#[tokio::test]
async fn execution_update_preserves_validation_and_legacy_acknowledgement() {
    let app = execution_creation_test_app(true);
    let payload = serde_json::json!({
        "testID": "42",
        "result": "PASS",
        "comments": "completed"
    });
    let unauthorized = app
        .clone()
        .oneshot(
            Request::put("/api/v1/device/test/execution/9")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::put("/api/v1/device/test/execution/9")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "{}"
    );

    let invalid = app
        .oneshot(
            Request::put("/api/v1/device/test/execution/9")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"testID":"42","result":"UNKNOWN"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let unavailable = execution_creation_test_app(false)
        .oneshot(
            Request::put("/api/v1/device/test/execution/not-numeric")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"testID":"not-numeric","result":"FAIL","log":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unavailable.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn log_routes_require_authentication() {
    let response = log_test_app()
        .oneshot(
            Request::get("/api/v1/device/log/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_log_applies_defaults_and_validates_payload() {
    let app = log_test_app();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/log/")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "referenceId": "device-1",
                        "data": {"message": "ready"}
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
    assert_eq!(body["type"], "general");
    assert_eq!(body["level"], "info");
    assert_eq!(body["referenceId"], "device-1");

    for payload in [
        serde_json::json!({"referenceId": "", "data": {}}),
        serde_json::json!({"referenceId": "device-1", "data": []}),
        serde_json::json!({"type": "unknown", "referenceId": "device-1", "data": {}}),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/device/log")
                    .header("authorization", "Bearer access")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn list_and_search_logs_preserve_pagination_and_filters() {
    let app = log_test_app();
    let response = app
        .clone()
        .oneshot(
            Request::get(
                "/api/v1/device/log?type=device&referenceId=device-1&level=warn&page=0&pageSize=200&sort=referenceId&order=ASC",
            )
            .header("authorization", "Bearer access")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["page"], 1);
    assert_eq!(body["pageSize"], 100);
    assert_eq!(body["logs"][0]["type"], "device");
    assert_eq!(body["logs"][0]["referenceId"], "device-1");
    assert_eq!(body["logs"][0]["level"], "warn");

    let missing = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/log/search")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let response = app
        .oneshot(
            Request::get("/api/v1/device/log/search?query=panic&page=2&pageSize=5")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["page"], 2);
    assert_eq!(body["pageSize"], 5);
    assert_eq!(body["logs"][0]["data"]["message"], "panic");
}
