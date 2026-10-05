use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

use super::*;

#[tokio::test]
async fn test_rail_plans_follow_pagination_and_filter() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/get_suites/9"))
        .and(query_param("limit", "250"))
        .and(query_param("offset", "0"))
        .and(header("authorization", "Basic dXNlcjprZXk="))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "suites": [{"id": 1, "name": "Other", "description": null}],
            "_links": {"next": "/api/v2/page2"}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "suites": [{"id": 2, "name": "Smoke", "description": "Device plan"}],
            "_links": {"next": null}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/get_sections/9"))
        .and(query_param("suite_id", "7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "sections": [{"id": 8, "name": "Core", "suite_id": 7, "display_order": 2, "description": null}],
            "_links": {"next": null}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/get_cases/9"))
        .and(query_param("suite_id", "7"))
        .and(query_param("section_id", "8"))
        .and(query_param("filter", "Boot"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "cases": [{"id": 9, "title": "Boot", "suite_id": 7, "section_id": 8, "display_order": 3, "priority_id": 2, "custom_steps": "%3Cb%3Erun.sh%3C%2Fb%3E", "custom_preconds": "Ready%20now", "labels": [{"title": "smoke"}]}],
            "_links": {"next": null}
        })))
        .mount(&server)
        .await;
    let catalog = TestRailCatalog::configured(TestsConfig {
        test_rail_base_url: format!("{}/", server.uri()),
        test_rail_api_version: "v2".to_owned(),
        test_rail_username: "user".to_owned(),
        test_rail_api_key: "key".to_owned(),
        test_rail_project_id: 9,
        test_rail_timeout_seconds: 5,
        qmetry_base_url: String::new(),
        qmetry_api_key: String::new(),
        jira_project_id: String::new(),
        qmetry_timeout_seconds: 5,
        test_logs_dir: "./tmp/test-logs".to_owned(),
        ..TestsConfig::default()
    })
    .unwrap()
    .unwrap();
    let plans = catalog.plans(Some("smoke")).await.unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].id, 2);
    assert!(plans[0].test_suits.is_empty());
    let suites = catalog.suites(7).await.unwrap();
    assert_eq!(suites[0].id, 8);
    assert_eq!(suites[0].plan_id, 7);
    assert_eq!(suites[0].order, 2);
    let cases = catalog.cases(7, 8, Some("Boot")).await.unwrap();
    assert_eq!(cases[0].script_file, "run.sh");
    assert_eq!(cases[0].pre_condition.as_deref(), Some("Ready now"));
    assert_eq!(cases[0].labels, vec!["smoke"]);
}

#[tokio::test]
async fn test_rail_creates_runs_and_bounds_script_downloads() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/add_run/9"))
        .and(header("authorization", "Basic dXNlcjprZXk="))
        .and(body_json(serde_json::json!({
            "suite_id": 7,
            "name": "Smoke run",
            "description": "Automated run",
            "include_all": false,
            "case_ids": [11, 12]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 42})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/scripts/run.sh"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"echo ok\r\n"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/scripts/large.sh"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 17]))
        .mount(&server)
        .await;
    let catalog = TestRailCatalog::configured(TestsConfig {
        test_rail_base_url: format!("{}/", server.uri()),
        test_rail_api_version: "v2".to_owned(),
        test_rail_username: "user".to_owned(),
        test_rail_api_key: "key".to_owned(),
        test_rail_project_id: 9,
        test_rail_timeout_seconds: 5,
        test_script_server: "local".to_owned(),
        local_test_script_server_url: format!("{}/", server.uri()),
        test_script_max_bytes: 16,
        ..TestsConfig::default()
    })
    .unwrap()
    .unwrap();
    let run_id = catalog
        .create_run(&TestRunCreation {
            suite_id: 7,
            name: "Smoke run".to_owned(),
            description: "Automated run".to_owned(),
            include_all: false,
            case_ids: vec![11, 12],
        })
        .await
        .unwrap();
    assert_eq!(run_id, 42);
    assert_eq!(
        catalog.test_script("scripts/run.sh").await.unwrap(),
        b"echo ok\r\n"
    );
    let error = catalog.test_script("scripts/large.sh").await.unwrap_err();
    assert!(error.to_string().contains("configured maximum size"));
    assert!(catalog.test_script("../secret").await.is_err());
}
