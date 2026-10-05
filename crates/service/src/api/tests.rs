use axum::{body::Body, http::Request};
use clap::Parser;
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::*;
use crate::{cli::Cli, config::AppConfig, observability::Metrics};

fn test_state(arguments: &[&str]) -> Arc<AppState> {
    let cli = Cli::try_parse_from(arguments).unwrap();
    Arc::new(AppState::without_dependencies(
        AppConfig::load(&cli).unwrap(),
        Metrics::new().unwrap(),
    ))
}

#[tokio::test]
async fn configured_cors_and_security_headers_are_enforced() {
    let mut state = match Arc::try_unwrap(test_state(&["farmcontroller"])) {
        Ok(state) => state,
        Err(_) => panic!("test state must be uniquely owned"),
    };
    state.config.server.cors_allowed_origins = vec!["https://farm.example".to_owned()];
    let app = router(Arc::new(state));

    for (origin, expected) in [
        ("https://farm.example", Some("https://farm.example")),
        ("https://untrusted.example", None),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::options("/health")
                    .header("origin", origin)
                    .header("access-control-request-method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            expected
        );
    }

    let response = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["x-frame-options"], "DENY");
    assert_eq!(
        response.headers()["referrer-policy"],
        "strict-origin-when-cross-origin"
    );
}

#[tokio::test]
async fn operational_routes_share_one_router() {
    let app = router(test_state(&["farmcontroller"]));
    for path in ["/health", "/ready", "/metrics", "/openapi.json"] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "path: {path}");
        assert!(response.headers().contains_key("x-request-id"));
    }
    let response = app
        .clone()
        .oneshot(Request::get("/openapi.json").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let document: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    for path in [
        "/ws",
        "/api/v1/device/build/upload",
        "/api/v1/device/build/upload/custom",
        "/api/v1/device/build/upload/tus",
        "/api/v1/device/build/upload/tus/{id}",
    ] {
        assert!(
            document["paths"].get(path).is_some(),
            "OpenAPI path: {path}"
        );
    }
    for schema in ["RelayRecord", "RelayChannelRecord", "RelayDeviceSummary"] {
        assert!(
            document["components"]["schemas"].get(schema).is_some(),
            "OpenAPI schema: {schema}"
        );
    }
    for schema in [
        "LogEntry",
        "LogList",
        "MessageResponse",
        "CompatibilityErrorResponse",
        "FaultyReportDetail",
        "FaultyReportRecord",
        "SuccessResponse",
        "BuildUploadResponse",
        "BuildFlagResponse",
        "ActiveExecutionRecord",
        "ExecutionCaseRecord",
        "ExecutionReportRecord",
        "BuildExecutionList",
        "DeviceDataResponse",
        "DeviceFlashingResponse",
        "DeviceHeartbeatResponse",
        "DeviceTopologyResponse",
        "AvailableRelayDevicesResponse",
        "LegacyRelayConfigurationResponse",
        "RelayConflictResponse",
        "TestCompletionResponse",
    ] {
        assert!(
            document["components"]["schemas"].get(schema).is_some(),
            "OpenAPI schema: {schema}"
        );
    }
    assert_eq!(
        document["paths"]["/api/v1/device/log/"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/LogList"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/notification/alerts/{id}/read"]["put"]["responses"]["200"]
            ["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/MessageResponse"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/faulty/report/{id}"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/FaultyReportDetail"
    );
    assert!(
        document["paths"]["/api/v1/device/faulty/report"]["post"]["requestBody"]["content"]
            .get("multipart/form-data")
            .is_some()
    );
    assert_eq!(
        document["paths"]["/api/v1/device/faulty/report/{id}/file"]["get"]["responses"]["200"]["content"]
            ["application/octet-stream"]["schema"]["type"],
        "string"
    );
    assert_eq!(
        document["components"]["schemas"]["CompatibilityErrorResponse"]["oneOf"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        document["paths"]["/api/v1/device/build/upload"]["post"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/BuildUploadResponse"
    );
    assert!(
        document["paths"]["/api/v1/device/build/upload/tus"]["options"]["responses"]["204"]
            ["headers"]
            .get("Tus-Max-Size")
            .is_some()
    );
    assert_eq!(
        document["paths"]["/api/v1/device/test/execution/progress/list"]["get"]["responses"]["200"]
            ["content"]["application/json"]["schema"]["items"]["$ref"],
        "#/components/schemas/ActiveExecutionRecord"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/test/execution/report/{id}"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/ExecutionReportRecord"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/{id}"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/DeviceDataResponse"
    );
    assert_eq!(
        document["paths"]["/api/v1/auth/users"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/ManagedUsersResponse"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/ws/send/message"]["post"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/UserMessageResponse"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/relay/devices/available"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/AvailableRelayDevicesResponse"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/relay/check-conflicts"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/RelayConflictResponse"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/{id}/test-completed"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/TestCompletionResponse"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/{id}/flashing"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/DeviceFlashingResponse"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/mapping-gen5"]["post"]["requestBody"]["content"]["application/json"]
            ["example"]["status"],
        "success"
    );
    for path in [
        "/api/v1/device/mapping-gen5",
        "/api/v1/device/flash-confirm",
        "/api/v1/device/flash-confirm-gen4",
    ] {
        let method = if path.ends_with("mapping-gen5") {
            "post"
        } else {
            "get"
        };
        assert_eq!(
            document["paths"][path][method]["responses"]["503"]["content"]["application/json"]["schema"]
                ["$ref"],
            "#/components/schemas/ErrorResponse",
            "callback persistence error for {method} {path}"
        );
    }
    for path in ["/api/v1/device/export", "/api/v1/device/test/export"] {
        assert_eq!(
            document["paths"][path]["get"]["responses"]["200"]["content"]["text/csv"]["schema"]["type"],
            "string",
            "CSV response body for {path}"
        );
        assert!(
            document["paths"][path]["get"]["responses"]["200"]["headers"]
                .get("Content-Disposition")
                .is_some(),
            "CSV attachment header for {path}"
        );
    }
    for (path, method, status) in [
        ("/api/v1/device/config/artifacts/default", "put", "503"),
        ("/api/v1/device/test/plan/{planID}/testcase", "get", "500"),
        (
            "/api/v1/device/test/plan/{planID}/suite/{suiteID}/testcase",
            "get",
            "503",
        ),
    ] {
        assert_eq!(
            document["paths"][path][method]["responses"][status]["content"]["application/json"]["schema"]
                ["$ref"],
            "#/components/schemas/ErrorResponse",
            "OpenAPI error response for {method} {path} {status}"
        );
    }
    let mut missing_descriptions = Vec::new();
    for (path, item) in document["paths"].as_object().unwrap() {
        for (method, operation) in item.as_object().unwrap() {
            if !matches!(
                method.as_str(),
                "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
            ) {
                continue;
            }
            assert!(
                operation["summary"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty()),
                "OpenAPI summary for {method} {path}"
            );
            if !operation["description"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            {
                missing_descriptions.push(format!("{method} {path}"));
            }
            assert!(
                operation["responses"].as_object().is_some_and(|responses| {
                    responses
                        .keys()
                        .any(|status| status.starts_with(['1', '2']))
                }),
                "OpenAPI success response for {method} {path}"
            );
        }
    }
    assert!(
        missing_descriptions.is_empty(),
        "OpenAPI descriptions missing for: {}",
        missing_descriptions.join(", ")
    );
    assert_eq!(
        document["paths"]["/api/v1/device/relay/{id}"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/RelayRecord"
    );
    assert_eq!(
        document["paths"]["/api/v1/device/relay/channel/{id}"]["get"]["responses"]["404"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/ErrorResponse"
    );
    for (path, status) in [
        ("/api/v1/device/analytics/state", "401"),
        ("/api/v1/device/analytics/execution/daily", "400"),
        ("/api/v1/device/analytics/test", "503"),
    ] {
        assert_eq!(
            document["paths"][path]["get"]["responses"][status]["content"]["application/json"]["schema"]
                ["$ref"],
            "#/components/schemas/ErrorResponse",
            "OpenAPI error response for {path} {status}"
        );
    }
    let response = app
        .oneshot(Request::get("/swagger").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(response.headers()["location"], "/swagger-ui/");
}

#[tokio::test]
async fn disabled_route_is_not_exposed() {
    let app = router(test_state(&["farmcontroller", "--disable", "metrics"]));
    let response = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("not_found"));
}

#[tokio::test]
async fn gateway_prefixes_and_tus_preflight_mount_directly() {
    let mut state = match Arc::try_unwrap(test_state(&["farmcontroller"])) {
        Ok(state) => state,
        Err(_) => panic!("test state must be uniquely owned"),
    };
    state
        .config
        .modules
        .enabled
        .extend([Module::Auth, Module::Device, Module::Events]);
    let app = router(Arc::new(state));

    let auth_response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/auth/signin")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(auth_response.status(), StatusCode::NOT_FOUND);

    let device_response = app
        .clone()
        .oneshot(Request::get("/api/v1/device/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(device_response.status(), StatusCode::UNAUTHORIZED);

    let tus_response = app
        .oneshot(
            Request::options("/api/v1/device/build/upload/tus")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tus_response.status(), StatusCode::OK);
    assert_eq!(tus_response.headers()["tus-resumable"], "1.0.0");
}
