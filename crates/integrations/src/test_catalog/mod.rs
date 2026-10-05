#[cfg(test)]
pub mod tests;

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::TestsConfig;

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestPlan {
    pub id: u64,
    pub name: String,
    pub description: Option<String>,
    pub test_suits: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestSuite {
    pub id: u64,
    pub name: String,
    pub plan_id: u64,
    pub order: i64,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestCase {
    pub id: u64,
    pub title: String,
    pub plan_id: u64,
    pub suite_id: u64,
    pub order: i64,
    pub priority_id: u64,
    pub script_file: String,
    pub pre_condition: Option<String>,
    pub labels: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TestResultUpdate {
    pub run_id: u64,
    pub case_id: u64,
    pub result: String,
    pub comment: String,
    pub version: Option<String>,
    pub defects: String,
}

#[derive(Clone, Debug)]
pub struct TestRunCreation {
    pub suite_id: u64,
    pub name: String,
    pub description: String,
    pub include_all: bool,
    pub case_ids: Vec<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum TestCatalogError {
    #[error("TestRail is not configured")]
    NotConfigured,
    #[error("TestRail request failed: {0}")]
    Request(String),
}

#[async_trait]
pub trait TestCatalog: Send + Sync {
    async fn plans(&self, filter: Option<&str>) -> Result<Vec<TestPlan>, TestCatalogError>;
    async fn suites(&self, plan_id: u64) -> Result<Vec<TestSuite>, TestCatalogError>;
    async fn cases(
        &self,
        plan_id: u64,
        suite_id: u64,
        filter: Option<&str>,
    ) -> Result<Vec<TestCase>, TestCatalogError>;
    async fn update_result(&self, update: &TestResultUpdate) -> Result<(), TestCatalogError>;
    async fn create_run(&self, request: &TestRunCreation) -> Result<u64, TestCatalogError> {
        let _ = request;
        Err(TestCatalogError::NotConfigured)
    }
    async fn delete_run(&self, run_id: u64) -> Result<(), TestCatalogError> {
        let _ = run_id;
        Err(TestCatalogError::NotConfigured)
    }
    async fn test_script(&self, path: &str) -> Result<Vec<u8>, TestCatalogError> {
        let _ = path;
        Err(TestCatalogError::NotConfigured)
    }
}

pub struct TestRailCatalog {
    config: TestsConfig,
    client: reqwest::Client,
    base: reqwest::Url,
}

impl TestRailCatalog {
    pub fn configured(
        config: TestsConfig,
    ) -> Result<Option<Arc<dyn TestCatalog>>, TestCatalogError> {
        if config.test_rail_base_url.trim().is_empty() {
            return Ok(None);
        }
        let base = reqwest::Url::parse(&config.test_rail_base_url)
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        let client =
            crate::external_http::client(Duration::from_secs(config.test_rail_timeout_seconds))
                .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        Ok(Some(Arc::new(Self {
            config,
            client,
            base,
        })))
    }

    async fn page(&self, url: reqwest::Url) -> Result<PlanPage, TestCatalogError> {
        self.client
            .get(url)
            .basic_auth(
                &self.config.test_rail_username,
                Some(&self.config.test_rail_api_key),
            )
            .send()
            .await
            .map_err(|error| TestCatalogError::Request(error.to_string()))?
            .error_for_status()
            .map_err(|error| TestCatalogError::Request(error.to_string()))?
            .json()
            .await
            .map_err(|error| TestCatalogError::Request(error.to_string()))
    }

    async fn json(&self, url: reqwest::Url) -> Result<serde_json::Value, TestCatalogError> {
        self.client
            .get(url)
            .basic_auth(
                &self.config.test_rail_username,
                Some(&self.config.test_rail_api_key),
            )
            .send()
            .await
            .map_err(|error| TestCatalogError::Request(error.to_string()))?
            .error_for_status()
            .map_err(|error| TestCatalogError::Request(error.to_string()))?
            .json()
            .await
            .map_err(|error| TestCatalogError::Request(error.to_string()))
    }
}

#[derive(Deserialize)]
struct PlanPage {
    #[serde(default)]
    suites: Vec<PlanWire>,
    #[serde(default, rename = "_links")]
    links: Links,
}
#[derive(Deserialize)]
struct PlanWire {
    id: u64,
    name: String,
    description: Option<String>,
}
#[derive(Default, Deserialize)]
struct Links {
    next: Option<String>,
}

#[async_trait]
impl TestCatalog for TestRailCatalog {
    async fn plans(&self, filter: Option<&str>) -> Result<Vec<TestPlan>, TestCatalogError> {
        let endpoint = format!(
            "api/{}/get_suites/{}?limit=250&offset=0",
            self.config.test_rail_api_version, self.config.test_rail_project_id
        );
        let mut next = Some(
            self.base
                .join(&endpoint)
                .map_err(|error| TestCatalogError::Request(error.to_string()))?,
        );
        let mut plans = Vec::new();
        while let Some(url) = next.take() {
            if url.origin() != self.base.origin() {
                return Err(TestCatalogError::Request(
                    "cross-origin pagination link rejected".to_owned(),
                ));
            }
            let page = self.page(url).await?;
            plans.extend(page.suites);
            next = page
                .links
                .next
                .as_deref()
                .map(|value| self.base.join(value))
                .transpose()
                .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        }
        let filter = filter
            .map(|value| value.trim_matches(['\'', '"']).to_lowercase())
            .filter(|value| !value.is_empty());
        Ok(plans
            .into_iter()
            .filter(|plan| {
                filter.as_ref().is_none_or(|filter| {
                    plan.name.to_lowercase().contains(filter)
                        || plan
                            .description
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(filter)
                })
            })
            .map(|plan| TestPlan {
                id: plan.id,
                name: plan.name,
                description: plan.description,
                test_suits: Vec::new(),
            })
            .collect())
    }

    async fn suites(&self, plan_id: u64) -> Result<Vec<TestSuite>, TestCatalogError> {
        let endpoint = format!(
            "api/{}/get_sections/{}?limit=250&offset=0&suite_id={plan_id}",
            self.config.test_rail_api_version, self.config.test_rail_project_id
        );
        let mut next = Some(
            self.base
                .join(&endpoint)
                .map_err(|error| TestCatalogError::Request(error.to_string()))?,
        );
        let mut suites = Vec::new();
        while let Some(url) = next.take() {
            if url.origin() != self.base.origin() {
                return Err(TestCatalogError::Request(
                    "cross-origin pagination link rejected".to_owned(),
                ));
            }
            let value = self.json(url).await?;
            for section in value
                .get("sections")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                suites.push(TestSuite {
                    id: section
                        .get("id")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default(),
                    name: section
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    plan_id: section
                        .get("suite_id")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(plan_id),
                    order: section
                        .get("display_order")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or_default(),
                    description: section
                        .get("description")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                });
            }
            next = value
                .pointer("/_links/next")
                .and_then(serde_json::Value::as_str)
                .map(|value| self.base.join(value))
                .transpose()
                .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        }
        Ok(suites)
    }

    async fn cases(
        &self,
        plan_id: u64,
        suite_id: u64,
        filter: Option<&str>,
    ) -> Result<Vec<TestCase>, TestCatalogError> {
        let endpoint = format!(
            "api/{}/get_cases/{}",
            self.config.test_rail_api_version, self.config.test_rail_project_id
        );
        let mut first = self
            .base
            .join(&endpoint)
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        first
            .query_pairs_mut()
            .append_pair("limit", "250")
            .append_pair("offset", "0")
            .append_pair("suite_id", &plan_id.to_string())
            .append_pair("section_id", &suite_id.to_string());
        if let Some(filter) = filter {
            first.query_pairs_mut().append_pair("filter", filter);
        }
        let mut next = Some(first);
        let mut cases = Vec::new();
        while let Some(url) = next.take() {
            if url.origin() != self.base.origin() {
                return Err(TestCatalogError::Request(
                    "cross-origin pagination link rejected".to_owned(),
                ));
            }
            let value = self.json(url).await?;
            for case in value
                .get("cases")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                cases.push(TestCase {
                    id: number(case, "id"),
                    title: text(case, "title"),
                    plan_id: number(case, "suite_id"),
                    suite_id: number(case, "section_id"),
                    order: case
                        .get("display_order")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or_default(),
                    priority_id: number(case, "priority_id"),
                    script_file: clean_text(
                        case.get("custom_steps").and_then(serde_json::Value::as_str),
                    )
                    .unwrap_or_default(),
                    pre_condition: clean_text(
                        case.get("custom_preconds")
                            .and_then(serde_json::Value::as_str),
                    ),
                    labels: case
                        .get("labels")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|label| {
                            label
                                .get("title")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned)
                        })
                        .collect(),
                });
            }
            next = value
                .pointer("/_links/next")
                .and_then(serde_json::Value::as_str)
                .map(|value| self.base.join(value))
                .transpose()
                .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        }
        Ok(cases)
    }

    async fn update_result(&self, update: &TestResultUpdate) -> Result<(), TestCatalogError> {
        let status_name = match update.result.as_str() {
            "PASS" => "passed",
            "FAIL" => "failed",
            "NOT EXECUTED" => "untested",
            "BLOCKED" => "blocked",
            "RETEST" => "retest",
            value => {
                return Err(TestCatalogError::Request(format!(
                    "invalid result type: {value}"
                )));
            }
        };
        let status_url = self
            .base
            .join(&format!(
                "api/{}/get_statuses",
                self.config.test_rail_api_version
            ))
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        let statuses = self.json(status_url).await?;
        let status_id = statuses
            .as_array()
            .into_iter()
            .flatten()
            .find(|status| {
                status.get("name").and_then(serde_json::Value::as_str) == Some(status_name)
            })
            .and_then(|status| status.get("id").and_then(serde_json::Value::as_u64))
            .filter(|id| (1..=5).contains(id))
            .ok_or_else(|| {
                TestCatalogError::Request(format!("invalid result type: {}", update.result))
            })?;
        let result_url = self
            .base
            .join(&format!(
                "api/{}/add_result_for_case/{}/{}",
                self.config.test_rail_api_version, update.run_id, update.case_id
            ))
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        self.client
            .post(result_url)
            .basic_auth(
                &self.config.test_rail_username,
                Some(&self.config.test_rail_api_key),
            )
            .json(&serde_json::json!({
                "status_id": status_id,
                "comment": update.comment,
                "version": update.version,
                "defects": update.defects
            }))
            .send()
            .await
            .map_err(|error| TestCatalogError::Request(error.to_string()))?
            .error_for_status()
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        Ok(())
    }

    async fn create_run(&self, request: &TestRunCreation) -> Result<u64, TestCatalogError> {
        let url = self
            .base
            .join(&format!(
                "api/{}/add_run/{}",
                self.config.test_rail_api_version, self.config.test_rail_project_id
            ))
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        let value: serde_json::Value = self
            .client
            .post(url)
            .basic_auth(
                &self.config.test_rail_username,
                Some(&self.config.test_rail_api_key),
            )
            .json(&serde_json::json!({
                "suite_id": request.suite_id,
                "name": request.name,
                "description": request.description,
                "include_all": request.include_all,
                "case_ids": (!request.include_all).then_some(&request.case_ids),
            }))
            .send()
            .await
            .map_err(|error| TestCatalogError::Request(error.to_string()))?
            .error_for_status()
            .map_err(|error| TestCatalogError::Request(error.to_string()))?
            .json()
            .await
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        value
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| TestCatalogError::Request("TestRail run response has no id".to_owned()))
    }

    async fn delete_run(&self, run_id: u64) -> Result<(), TestCatalogError> {
        let url = self
            .base
            .join(&format!(
                "api/{}/delete_run/{run_id}",
                self.config.test_rail_api_version
            ))
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        self.client
            .post(url)
            .basic_auth(
                &self.config.test_rail_username,
                Some(&self.config.test_rail_api_key),
            )
            .send()
            .await
            .map_err(|error| TestCatalogError::Request(error.to_string()))?
            .error_for_status()
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        Ok(())
    }

    async fn test_script(&self, path: &str) -> Result<Vec<u8>, TestCatalogError> {
        use futures_util::StreamExt;

        let path = path.trim().trim_start_matches('/');
        if path.is_empty()
            || path
                .split('/')
                .any(|segment| matches!(segment, "" | "." | ".."))
        {
            return Err(TestCatalogError::Request(
                "invalid test script path".to_owned(),
            ));
        }
        let mut request = if self.config.test_script_server.eq_ignore_ascii_case("local") {
            let base = reqwest::Url::parse(&self.config.local_test_script_server_url)
                .map_err(|error| TestCatalogError::Request(error.to_string()))?;
            self.client.get(
                base.join(path)
                    .map_err(|error| TestCatalogError::Request(error.to_string()))?,
            )
        } else {
            if self.config.gitlab_base_url.trim().is_empty()
                || self.config.gitlab_project_id.trim().is_empty()
            {
                return Err(TestCatalogError::NotConfigured);
            }
            let base = reqwest::Url::parse(&self.config.gitlab_base_url)
                .map_err(|error| TestCatalogError::Request(error.to_string()))?;
            let encoded_path =
                percent_encoding::utf8_percent_encode(path, percent_encoding::NON_ALPHANUMERIC);
            let endpoint = format!(
                "api/v4/projects/{}/repository/files/{encoded_path}/raw",
                self.config.gitlab_project_id
            );
            self.client
                .get(
                    base.join(&endpoint)
                        .map_err(|error| TestCatalogError::Request(error.to_string()))?,
                )
                .query(&[("ref", &self.config.gitlab_branch)])
        };
        if !self.config.gitlab_access_token.is_empty()
            && !self.config.test_script_server.eq_ignore_ascii_case("local")
        {
            request = request.header("PRIVATE-TOKEN", &self.config.gitlab_access_token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| TestCatalogError::Request(error.to_string()))?
            .error_for_status()
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        if response
            .content_length()
            .is_some_and(|length| length > self.config.test_script_max_bytes as u64)
        {
            return Err(TestCatalogError::Request(
                "test script exceeds configured maximum size".to_owned(),
            ));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| TestCatalogError::Request(error.to_string()))?;
            if bytes.len().saturating_add(chunk.len()) > self.config.test_script_max_bytes {
                return Err(TestCatalogError::Request(
                    "test script exceeds configured maximum size".to_owned(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

fn number(value: &serde_json::Value, field: &str) -> u64 {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default()
}
fn text(value: &serde_json::Value, field: &str) -> String {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn clean_text(value: Option<&str>) -> Option<String> {
    let value = value?;
    let decoded = percent_encoding::percent_decode_str(value).decode_utf8_lossy();
    let without_tags = regex::Regex::new(r"<[^>]*>")
        .expect("static regex")
        .replace_all(&decoded, " ");
    let cleaned = without_tags
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!cleaned.is_empty()).then_some(cleaned)
}
