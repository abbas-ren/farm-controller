use super::*;
#[tokio::test]
async fn build_list_requires_auth_and_preserves_frontend_shape() {
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
            Request::get("/api/v1/device/build")
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
    let mut events = state.event_publisher.subscribe();
    let authenticated = api::router(Arc::new(state));
    let response = authenticated
        .clone()
        .oneshot(
            Request::get("/api/v1/device/build?page=0&limit=20&flagged=true")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["builds"][0]["id"], "build-1");
    assert_eq!(body["totalCount"], 1);
    assert_eq!(body["currentPage"], 1);
    assert_eq!(body["totalPages"], 1);
    assert_eq!(body["requestedCount"], 1);

    for (build_id, expected) in [
        ("build-1", StatusCode::OK),
        ("missing", StatusCode::NOT_FOUND),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/device/build/{build_id}"))
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
            assert_eq!(body["id"], "build-1");
            assert_eq!(body["version"], "1.2.3");
        }
    }

    let response = authenticated
        .clone()
        .oneshot(
            Request::get("/api/v1/device/build/filters")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["deviceTypes"], serde_json::json!(["x5h"]));
    assert_eq!(body["deviceFamilies"], serde_json::json!(["Gen5"]));

    for (payload, expected) in [
        (serde_json::json!({"fileCount": 3}), StatusCode::CREATED),
        (serde_json::json!({"fileCount": 0}), StatusCode::BAD_REQUEST),
        (serde_json::json!({}), StatusCode::BAD_REQUEST),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::post("/api/v1/device/build/upload/init")
                    .header("authorization", "Bearer access")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::CREATED {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body["uploadId"], Uuid::nil().to_string());
        }
    }

    let multipart = |include_tag: bool| {
        let tag = if include_tag {
            "--boundary\r\nContent-Disposition: form-data; name=\"tag\"\r\n\r\nnightly\r\n"
        } else {
            ""
        };
        format!(
            "--boundary\r\nContent-Disposition: form-data; name=\"uploadId\"\r\n\r\n{}\r\n{tag}--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x5h__1.2.3.zip\"\r\nContent-Type: application/zip\r\n\r\nzip\r\n--boundary--\r\n",
            Uuid::nil()
        )
    };
    for (path, include_tag, expected) in [
        ("/api/v1/device/build/upload/custom", true, StatusCode::OK),
        (
            "/api/v1/device/build/upload/custom",
            false,
            StatusCode::BAD_REQUEST,
        ),
        ("/api/v1/device/build/upload", false, StatusCode::OK),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::post(path)
                    .header("authorization", "Bearer access")
                    .header("content-type", "multipart/form-data; boundary=boundary")
                    .body(Body::from(multipart(include_tag)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body["message"], "Upload successful");
            assert_eq!(body["filename"], "x5h__1.2.3.zip");
            let progress = events.recv().await.unwrap();
            assert_eq!(progress.event, "upload_progress");
            assert_eq!(progress.payload["percent"], 0);
            assert_eq!(progress.room.as_deref(), Some("user:system"));
            let progress = events.recv().await.unwrap();
            assert_eq!(progress.event, "upload_progress");
            assert_eq!(progress.payload["percent"], 100);
            let complete = events.recv().await.unwrap();
            assert_eq!(complete.event, "upload_complete");
            assert_eq!(complete.payload["uploadId"], Uuid::nil().to_string());
            assert_eq!(complete.payload["fileName"], "x5h__1.2.3.zip");
            let uploaded = events.recv().await.unwrap();
            assert_eq!(uploaded.event, "build_uploaded");
            assert_eq!(uploaded.payload["deviceType"], "x5h");
            let alert = events.recv().await.unwrap();
            assert_eq!(alert.event, "alert");
            assert_eq!(alert.payload["message"]["title"], "Build Upload Success");
        }
    }

    for (build_id, payload, expected) in [
        (
            "build-1",
            serde_json::json!({"isFaulty": true}),
            StatusCode::OK,
        ),
        ("build-1", serde_json::json!({}), StatusCode::BAD_REQUEST),
        (
            "missing",
            serde_json::json!({"isFaulty": false}),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::put(format!("/api/v1/device/build/{build_id}/flag"))
                    .header("authorization", "Bearer access")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body, serde_json::json!({"id": "build-1", "isFaulty": true}));
            let event = events.recv().await.unwrap();
            assert_eq!(event.event, "build_flagged");
            assert_eq!(event.payload["releaseId"], "build-1");
            assert_eq!(event.payload["version"], "v1.2.3");
            let alert = events.recv().await.unwrap();
            assert_eq!(alert.event, "alert");
            assert_eq!(alert.payload["message"]["title"], "Build Flagged");
        }
    }

    for (build_id, expected) in [
        ("build-1", StatusCode::NO_CONTENT),
        ("missing", StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::delete(format!("/api/v1/device/build/{build_id}"))
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::NO_CONTENT {
            let event = events.recv().await.unwrap();
            assert_eq!(event.event, "build_deleted");
            assert_eq!(event.payload["deviceType"], "x5h");
            let alert = events.recv().await.unwrap();
            assert_eq!(alert.event, "alert");
            assert_eq!(alert.payload["message"]["title"], "Build Deleted");
        }
    }
}

#[tokio::test]
async fn tus_upload_preserves_offsets_and_protocol_headers() {
    let directory = tempfile::tempdir().unwrap();
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    config.device.build_upload_dir = directory.path().to_string_lossy().into_owned();
    config.device.build_upload_max_bytes = 16;
    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/build/upload/tus")
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .header("upload-length", "5")
                .header(
                    "upload-metadata",
                    "filename YnVpbGQuYmlu,uploadId YmF0Y2gtMQ==",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["tus-resumable"], "1.0.0");
    let location = response.headers()["location"].to_str().unwrap().to_owned();
    assert!(location.starts_with("/api/v1/device/build/upload/tus/"));

    let response = app
        .clone()
        .oneshot(
            Request::head(&location)
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["upload-offset"], "0");
    assert_eq!(response.headers()["upload-length"], "5");

    let wrong_offset = app
        .clone()
        .oneshot(
            Request::patch(&location)
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .header("content-type", "application/offset+octet-stream")
                .header("upload-offset", "2")
                .body(Body::from("abc"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_offset.status(), StatusCode::CONFLICT);

    let response = app
        .clone()
        .oneshot(
            Request::patch(&location)
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .header("content-type", "application/offset+octet-stream")
                .header("upload-offset", "0")
                .body(Body::from("abc"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(response.headers()["upload-offset"], "3");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/device/build/upload/tus")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["tus-version"], "1.0.0");

    let create_final = |metadata: &'static str| {
        Request::post("/api/v1/device/build/upload/tus")
            .header("authorization", "Bearer access")
            .header("x-user-id", "user-1")
            .header("tus-resumable", "1.0.0")
            .header("upload-length", "3")
            .header("upload-metadata", metadata)
            .body(Body::empty())
            .unwrap()
    };
    let response = app
        .clone()
        .oneshot(create_final(
            "filename eDVoX18xLjIuMy56aXA=,uploadId MDAwMDAwMDAtMDAwMC0wMDAwLTAwMDAtMDAwMDAwMDAwMDAw",
        ))
        .await
        .unwrap();
    let complete_location = response.headers()["location"].to_str().unwrap().to_owned();
    let response = app
        .clone()
        .oneshot(
            Request::patch(&complete_location)
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .header("content-type", "application/offset+octet-stream")
                .header("upload-offset", "0")
                .body(Body::from("zip"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(response.headers()["upload-offset"], "3");

    for (event, percent) in [
        ("upload_progress", Some(0)),
        ("upload_progress", Some(60)),
        ("upload_progress", Some(0)),
        ("upload_progress", Some(100)),
        ("upload_complete", None),
        ("build_uploaded", None),
        ("alert", None),
    ] {
        let received = events.recv().await.unwrap();
        assert_eq!(received.event, event);
        if let Some(percent) = percent {
            assert_eq!(received.payload["percent"], percent);
            assert_eq!(received.room.as_deref(), Some("user:user-1"));
        }
    }

    let response = app
        .clone()
        .oneshot(create_final("filename eDVoX18xLjIuMy56aXA="))
        .await
        .unwrap();
    let invalid_location = response.headers()["location"].to_str().unwrap().to_owned();
    let response = app
        .clone()
        .oneshot(
            Request::patch(&invalid_location)
                .header("authorization", "Bearer access")
                .header("tus-resumable", "1.0.0")
                .header("content-type", "application/offset+octet-stream")
                .header("upload-offset", "0")
                .body(Body::from("zip"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = app
        .clone()
        .oneshot(
            Request::head(&invalid_location)
                .header("authorization", "Bearer access")
                .header("tus-resumable", "1.0.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["upload-offset"], "0");

    let response = app
        .clone()
        .oneshot(create_final(
            "filename ZmFpbC56aXA=,uploadId MDAwMDAwMDAtMDAwMC0wMDAwLTAwMDAtMDAwMDAwMDAwMDAw",
        ))
        .await
        .unwrap();
    let failed_location = response.headers()["location"].to_str().unwrap().to_owned();
    let response = app
        .clone()
        .oneshot(
            Request::patch(&failed_location)
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .header("content-type", "application/offset+octet-stream")
                .header("upload-offset", "0")
                .body(Body::from("zip"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    for (event, percent) in [
        ("upload_progress", Some(0)),
        ("upload_progress", Some(100)),
        ("upload_error", None),
    ] {
        let received = events.recv().await.unwrap();
        assert_eq!(received.event, event);
        if let Some(percent) = percent {
            assert_eq!(received.payload["percent"], percent);
        } else {
            assert_eq!(received.payload["message"], "upload processing failed");
            assert_eq!(received.room.as_deref(), Some("user:user-1"));
        }
    }
    let response = app
        .oneshot(
            Request::head(&failed_location)
                .header("authorization", "Bearer access")
                .header("tus-resumable", "1.0.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["upload-offset"], "0");
}
