use super::*;
#[tokio::test]
async fn test_execution_detail_supports_latest_and_configured_logs() {
    let directory = tempfile::tempdir().unwrap();
    tokio::fs::write(directory.path().join("test-1.json"), br#"["started"]"#)
        .await
        .unwrap();
    let report_directory = directory.path().join("x5h/v1.2.3/test-1");
    tokio::fs::create_dir_all(&report_directory).await.unwrap();
    tokio::fs::write(report_directory.join("report.html"), "<h1>Report</h1>")
        .await
        .unwrap();
    tokio::fs::write(report_directory.join("case.log"), "case output")
        .await
        .unwrap();
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    config.tests.test_logs_dir = directory.path().to_string_lossy().into_owned();
    config.reports.test_results_dir = directory.path().to_path_buf();
    let metrics = Metrics::new().unwrap();
    let reports = ReportService::new(
        config.reports.clone(),
        Arc::new(FakeReportDataSource),
        metrics.clone(),
    );
    let app = api::router(Arc::new(
        AppState::with_identity_provider(config, metrics, Arc::new(AcceptingIdentityProvider))
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }))
            .with_reports(reports),
    ));
    for (id, expected) in [
        ("test-1", StatusCode::OK),
        ("latest", StatusCode::OK),
        ("missing", StatusCode::NOT_FOUND),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/device/test/execution/{id}"))
                    .header("authorization", "Bearer access")
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
            assert_eq!(body["testId"], "test-1");
            assert_eq!(body["logs"], serde_json::json!(["started"]));
        }
    }
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/test-1?table=true")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["testId"], "test-1");
    assert_eq!(body["durationSeconds"], 60);
    assert_eq!(body["total"], 1);
    assert_eq!(body["testCases"][0]["testCaseId"], 11);
    assert_eq!(body["analytics"]["allTime"]["passed"], 1);
    assert_eq!(body["logs"], serde_json::json!(["started"]));

    for (test_id, expected) in [
        ("test-1", StatusCode::OK),
        ("missing", StatusCode::NOT_FOUND),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/device/test/execution/single/{test_id}"))
                    .header("authorization", "Bearer access")
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
            assert_eq!(body["testCases"][0]["testCaseId"], 11);
        }
    }
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/list/test-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body, serde_json::json!(["11", "12"]));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/device/device-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["logs"], serde_json::json!(["started"]));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/progress/list")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["deviceId"], "device-1");
    assert_eq!(body[0]["status"], "in_progress");

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution?page=0&limit=20&sortBy=deviceType&desc=false&search=x5")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["currentPage"], 1);
    assert_eq!(body["totalPages"], 1);
    assert_eq!(body["data"][0]["testId"], "test-1");
    assert_eq!(body["data"][0]["testCases"], serde_json::json!({}));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/cases/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["testCaseId"], 11);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/testcase/1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/logs/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/plain");
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"test-test-1-logs.txt\""
    );
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "started"
    );

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/report/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/report/test-1/html")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "<h1>Report</h1>"
    );

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/testcase/1/log")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"case.log\""
    );
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "case output"
    );

    let response = app
        .clone()
        .oneshot(
            Request::put("/api/v1/device/test/execution/report/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["testExecutionId"], "test-1");
    assert_eq!(body["status"], "generating");

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/build/build-1?limit=2&offset=1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["limit"], 2);
    assert_eq!(body["offset"], 1);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/results/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::put("/api/v1/device/test/cancel/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["message"], "  Test execution cancelled successfully");
}
