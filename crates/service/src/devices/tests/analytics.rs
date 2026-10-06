use super::*;
#[tokio::test]
async fn device_state_analytics_preserve_auth_and_grouping_contracts() {
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

    let unauthorized = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/state")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let state = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/state")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(state.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&state.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["total"], 2);
    assert_eq!(body["stateCount"]["busy"], 1);
    assert_eq!(body["stateCount"]["free"], 1);

    let detailed = app
        .oneshot(
            Request::get("/api/v1/device/analytics/state/detailed")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detailed.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&detailed.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["Gen5"][0]["deviceId"], "device-1");
    assert_eq!(body["Gen5"][0]["deviceType"], "Racer");
    assert_eq!(body["Gen5"][0]["state"], "free");
}

#[tokio::test]
async fn execution_activity_analytics_preserve_legacy_shapes() {
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

    let recent = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/execution?count=2")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(recent.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&recent.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["testId"], "test-1");
    assert_eq!(body[0]["totalTestCases"], 2);

    let daily = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/execution/daily?from=2026-10-01&to=2026-10-02")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(daily.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&daily.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["date"], "2026-10-01");
    assert_eq!(body[0]["totalDurationSeconds"], 60);

    let execution = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/execution/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(execution.status(), StatusCode::OK);

    let missing = app
        .oneshot(
            Request::get("/api/v1/device/analytics/execution/missing")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value =
        serde_json::from_slice(&missing.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["message"], "Test execution not found");
}

#[tokio::test]
async fn build_analytics_preserve_comparison_and_performance_shapes() {
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

    for path in [
        "/api/v1/device/analytics/builds/comparison?count=2",
        "/api/v1/device/analytics/builds/performance?count=2",
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
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body[0]["buildId"], "build-1");
    }

    let comparison = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/builds/comparison/build-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(comparison.status(), StatusCode::OK);

    let missing = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/builds/comparison/missing")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let performance = app
        .oneshot(
            Request::get("/api/v1/device/analytics/builds/performance/build-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(performance.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&performance.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(body["passedTestCasePercentage"], 50.0);
    assert_eq!(body["uniqueDeviceCount"], 1);
}

#[tokio::test]
async fn device_usage_analytics_preserve_daily_and_period_shapes() {
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

    let daily = app
        .clone()
        .oneshot(
            Request::get(
                "/api/v1/device/analytics/usage/daily?day=2026-10-02&deviceId=device-1&deviceFamily=Gen5",
            )
            .header("authorization", "Bearer access")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(daily.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&daily.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["totalDevices"], 1);
    assert_eq!(body["totalSeconds"], 3600);
    assert_eq!(body["states"]["free"], 3600);

    let invalid = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/usage/daily?day=10-02-2026")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let summary = app
        .oneshot(
            Request::get("/api/v1/device/analytics/usage/summary")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(summary.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&summary.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["weekly"].as_array().unwrap().len(), 4);
    assert_eq!(body["monthly"].as_array().unwrap().len(), 4);
    assert!(body["weekly"][0]["weekStart"].is_string());
    assert!(body["monthly"][3]["monthEnd"].is_string());
}
