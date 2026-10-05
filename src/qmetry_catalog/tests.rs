use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

use super::*;

#[tokio::test]
async fn qmetry_cases_preserve_grouped_and_folder_shapes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/PRJ/testcase-folders"))
        .and(header("apikey", "secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": 7, "children": [{"id": 8}]}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/testcases/search/"))
        .and(query_param("fields", "seqNo,key,version,summary,priority,status"))
        .and(body_json(serde_json::json!({"filter": {"projectId": "PRJ", "folderId": 8}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total": 1,
            "data": [{"id": "case-1", "summary": "Boot", "key": "TC-1", "seqNo": 1, "version": {"versionNo": 1}, "customFields": {"script": {"name": "Script File", "value": "run.sh"}}, "labels": [{"title": "smoke"}]}]
        })))
        .mount(&server).await;
    let catalog = HttpQmetryCatalog::configured(TestsConfig {
        qmetry_base_url: format!("{}/", server.uri()),
        qmetry_api_key: "secret".to_owned(),
        jira_project_id: "PRJ".to_owned(),
        qmetry_timeout_seconds: 5,
        ..TestsConfig::default()
    })
    .unwrap()
    .unwrap();
    let grouped = catalog.cases_by_plan(7).await.unwrap();
    assert_eq!(grouped["8"][0]["script_file"], "run.sh");
    assert_eq!(grouped["8"][0]["labels"], serde_json::json!(["smoke"]));
    let folder = catalog.cases_by_folder(8).await.unwrap();
    assert_eq!(folder[0]["id"], "case-1");
    assert!(folder[0].get("labels").is_none());
}
