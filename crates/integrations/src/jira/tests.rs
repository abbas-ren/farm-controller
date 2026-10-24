//! Wiremock coverage for Jira create-vs-comment defect filing.

use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

use super::*;

/// PURPOSE: Build TestsConfig pointed at a mock Jira server.
/// USE: Shared fixture for create/comment tests.
fn config(server: &MockServer) -> TestsConfig {
    TestsConfig {
        jira_base_url: server.uri(),
        jira_endpoint: "rest/api/2".to_owned(),
        jira_project_key: "RACER".to_owned(),
        jira_api_token: "secret".to_owned(),
        jira_timeout_seconds: 5,
        jira_script_base_url: "https://git.example/files".to_owned(),
        ..TestsConfig::default()
    }
}

/// PURPOSE: Sample defect payload used by both create and comment paths.
/// USE: shared fixture for JiraClient::add_defect tests.
fn defect() -> JiraDefect {
    JiraDefect {
        summary: "Case failed".to_owned(),
        script_file: "suite/case.sh".to_owned(),
        device_family: "Gen4".to_owned(),
        build: "build-1".to_owned(),
        test_plan: "Release".to_owned(),
        test_suite: "Sanity".to_owned(),
        execution_id: "test-1".to_owned(),
        test_rail_run: "run-2".to_owned(),
        comments: "FAIL".to_owned(),
    }
}

/// PURPOSE: Empty search → POST /issue and return browse URL.
/// USE: regression for Jira create-vs-comment filing when no open issue matches.
#[tokio::test]
async fn creates_bug_when_no_open_issue_matches() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .and(query_param("fields", "key,summary,status,self"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"issues": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({"key": "RACER-7"})),
        )
        .mount(&server)
        .await;

    let client = JiraClient::configured(&config(&server)).unwrap().unwrap();
    assert_eq!(
        client.add_defect(&defect()).await.unwrap(),
        format!("{}/browse/RACER-7", server.uri())
    );
}

/// PURPOSE: Matching open issue → comment instead of creating a duplicate.
/// USE: regression for Jira create-vs-comment filing when an open issue matches.
#[tokio::test]
async fn comments_on_existing_open_issue() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"issues": [{"key": "RACER-3"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/RACER-3/comment"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let client = JiraClient::configured(&config(&server)).unwrap().unwrap();
    assert_eq!(
        client.add_defect(&defect()).await.unwrap(),
        format!("{}/browse/RACER-3", server.uri())
    );
}
