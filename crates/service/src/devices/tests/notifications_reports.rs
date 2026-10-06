use super::*;
#[tokio::test]
async fn test_analytics_preserve_type_plan_and_daily_contracts() {
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

    let aggregate = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/test?type=execution")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(aggregate.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&aggregate.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["allTime"]["total"], 2);
    assert!(body["weekly"].is_array());
    assert!(body["monthly"].is_array());

    let in_progress = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/test?type=inProgress")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(in_progress.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&in_progress.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(body[0]["executed"], 1);

    let invalid = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/test?type=unknown")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let plan = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/test/plan")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(plan.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&plan.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["total"], 4);
    assert_eq!(body["cancelled"], 1);

    let daily = app
        .oneshot(
            Request::get("/api/v1/device/analytics/test/daily?days=8")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(daily.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&daily.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["totalExecutionSeconds"], 60);
}

#[tokio::test]
async fn notification_routes_preserve_visibility_and_read_contracts() {
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
            Request::get("/api/v1/device/notification/alerts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let admin = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/notification/alerts?page=2&limit=5")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&admin.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["currentPage"], 2);
    assert_eq!(body["data"][0]["deviceId"], "device-1");
    assert!(body.get("totalUnreadCount").is_none());

    let all = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/notification/alerts/all?from=invalid")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(all.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&all.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["totalUnreadCount"], 1);

    let missing = app
        .clone()
        .oneshot(
            Request::put("/api/v1/device/notification/alerts/missing/read")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let legacy = app
        .clone()
        .oneshot(
            Request::put("/api/v1/device/notification/alerts/read/missing")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::OK);

    let all_read = app
        .oneshot(
            Request::put("/api/v1/device/notification/alerts/all/read")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(all_read.status(), StatusCode::OK);
}

#[tokio::test]
async fn faulty_report_routes_preserve_create_admin_and_status_contracts() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let attachment = temporary.path().join("attachment.txt");
    std::fs::write(&attachment, "fault details").unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    config.device.faulty_report_upload_dir = temporary.path().display().to_string();
    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(vec![format!("faulty-file:{}", attachment.display())]),
    }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let token = "Bearer e30.eyJzdWIiOiJ1c2VyLTEifQ.e30";
    let boundary = "faulty-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"deviceType\"\r\n\r\nRacer\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"deviceFamily\"\r\n\r\nGen5\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"releaseId\"\r\n\r\n11111111-1111-4111-8111-111111111111\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"description\"\r\n\r\nIntermittent failure\r\n--{boundary}--\r\n"
    );
    let created = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/faulty/report")
                .header("authorization", token)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let event = events.recv().await.unwrap();
    assert_eq!(event.event, "alert");
    assert_eq!(event.payload["subtype"], "device-alert");
    assert_eq!(event.payload["message"]["title"], "Faulty Report");
    assert_eq!(event.payload["message"]["type"], "warning");
    assert_eq!(event.payload["message"]["buildVersion"], "v1");

    let list = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/faulty/report")
                .header("authorization", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);

    let id = "11111111-1111-4111-8111-111111111111";
    let detail = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/device/faulty/report/{id}"))
                .header("authorization", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&detail.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["buildVersion"], "v1");
    assert_eq!(body["user"]["userName"], "farm.user");

    let file = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/device/faulty/report/{id}/file"))
                .header("authorization", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(file.status(), StatusCode::OK);
    assert_eq!(file.headers()["content-type"], "text/plain; charset=utf-8");
    assert!(
        file.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("attachment.txt")
    );
    assert_eq!(
        file.into_body().collect().await.unwrap().to_bytes(),
        "fault details"
    );

    let invalid = app
        .clone()
        .oneshot(
            Request::patch(format!("/api/v1/device/faulty/report/{id}/status"))
                .header("authorization", token)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"status":"pending"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let approved = app
        .clone()
        .oneshot(
            Request::patch(format!("/api/v1/device/faulty/report/{id}/status"))
                .header("authorization", token)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"status":"approved"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approved.status(), StatusCode::OK);

    let deleted = app
        .oneshot(
            Request::delete(format!("/api/v1/device/faulty/report/{id}"))
                .header("authorization", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);
}
