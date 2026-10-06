use super::*;
#[tokio::test]
async fn test_export_requires_auth_and_returns_csv_attachment() {
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
            Request::get("/api/v1/device/test/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            config,
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(repository),
    ));
    let response = app
        .oneshot(
            Request::get("/api/v1/device/test/export")
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
            .starts_with("attachment; filename=\"test-executions-")
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
    assert!(body.starts_with("S.No,Test Name,Status,Device,Duration"));
    assert!(body.contains("1,Smoke,Completed,x5h,1 Min,1,1,0,,100"));
}

#[tokio::test]
async fn test_plan_route_requires_auth_and_returns_bare_array() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let config = AppConfig::load(&cli).unwrap();
    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    }))
    .with_test_catalog(Arc::new(FakeTestCatalog));
    let state = state.with_qmetry_catalog(Arc::new(FakeQmetryCatalog));
    let app = api::router(Arc::new(state));
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/plan?filter=Smoke&buildId=b1&deviceId=d1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["id"], 7);
    assert_eq!(body[0]["name"], "Smoke");
    assert_eq!(body[0]["testSuits"], serde_json::json!([]));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/suite?planId=7")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["id"], 8);
    assert_eq!(body[0]["planId"], 7);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/suite")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/testcase?planId=7&suiteId=8&filter=Boot")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["id"], 9);
    assert_eq!(body[0]["scriptFile"], "run.sh");
    assert_eq!(body[0]["labels"], serde_json::json!(["smoke"]));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/testcase?planId=7")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/plan/7/testcase")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["8"][0]["labels"], serde_json::json!(["smoke"]));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/plan/7/suite/8/testcase")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["id"], "case-1");

    let response = app
        .oneshot(
            Request::get("/api/v1/device/test/plan/1/testcase")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
