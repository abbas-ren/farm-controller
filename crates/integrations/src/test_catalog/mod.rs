//! TestRail (and local/GitLab script) catalog integration.
//!
//! Ports TestRail plan/suite/case/run APIs and script fetching from the old
//! device-service TestService / GitLabFileFetcher helpers. The [`TestCatalog`]
//! trait is the service-facing port; [`TestRailCatalog`] is the HTTP adapter.

#[cfg(test)]
pub mod tests;

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::TestsConfig;

/// TestRail suite treated as a farm "plan" in API responses.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestPlan {
    pub id: u64,
    pub name: String,
    pub description: Option<String>,
    pub test_suits: Vec<serde_json::Value>,
}

/// TestRail section under a plan/suite.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestSuite {
    pub id: u64,
    pub name: String,
    pub plan_id: u64,
    pub order: i64,
    pub description: Option<String>,
}

/// Normalized case row returned to the FarmController API.
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

/// Payload for posting a case result back to TestRail.
#[derive(Clone, Debug)]
pub struct TestResultUpdate {
    pub run_id: u64,
    pub case_id: u64,
    pub result: String,
    pub comment: String,
    pub version: Option<String>,
    pub defects: String,
}

/// Payload for creating a TestRail run for selected cases.
#[derive(Clone, Debug)]
pub struct TestRunCreation {
    pub suite_id: u64,
    pub name: String,
    pub description: String,
    pub include_all: bool,
    pub case_ids: Vec<u64>,
}

/// Catalog configuration and HTTP failures.
#[derive(Debug, thiserror::Error)]
pub enum TestCatalogError {
    /// Catalog is disabled or incomplete in config.
    #[error("TestRail is not configured")]
    NotConfigured,
    /// HTTP or JSON failure talking to TestRail/GitLab/local script server.
    #[error("TestRail request failed: {0}")]
    Request(String),
}

/// Port for reading plans/suites/cases and writing results/runs/scripts.
#[async_trait]
pub trait TestCatalog: Send + Sync {
    /// List plans (TestRail suites), optionally filtered by name/description.
    async fn plans(&self, filter: Option<&str>) -> Result<Vec<TestPlan>, TestCatalogError>;
    /// List suites/sections for a plan.
    async fn suites(&self, plan_id: u64) -> Result<Vec<TestSuite>, TestCatalogError>;
    /// List cases for a plan+suite with optional remote filter.
    async fn cases(
        &self,
        plan_id: u64,
        suite_id: u64,
        filter: Option<&str>,
    ) -> Result<Vec<TestCase>, TestCatalogError>;
    /// Post a case result to the external catalog.
    async fn update_result(&self, update: &TestResultUpdate) -> Result<(), TestCatalogError>;
    /// Create a run; default returns [`TestCatalogError::NotConfigured`].
    async fn create_run(&self, request: &TestRunCreation) -> Result<u64, TestCatalogError> {
        let _ = request;
        Err(TestCatalogError::NotConfigured)
    }
    /// Delete a run; default returns [`TestCatalogError::NotConfigured`].
    async fn delete_run(&self, run_id: u64) -> Result<(), TestCatalogError> {
        let _ = run_id;
        Err(TestCatalogError::NotConfigured)
    }
    /// Fetch script bytes; default returns [`TestCatalogError::NotConfigured`].
    async fn test_script(&self, path: &str) -> Result<Vec<u8>, TestCatalogError> {
        let _ = path;
        Err(TestCatalogError::NotConfigured)
    }
}

/// HTTP TestRail client implementing [`TestCatalog`].
pub struct TestRailCatalog {
    config: TestsConfig,
    client: reqwest::Client,
    base: reqwest::Url,
}

impl TestRailCatalog {
    /// PURPOSE: Build a TestRail catalog when base URL is configured.
    /// USE: Service bootstrap; returns `Ok(None)` when TestRail is disabled.
    pub fn configured(
        config: TestsConfig,
    ) -> Result<Option<Arc<dyn TestCatalog>>, TestCatalogError> {
        if config.test_rail_base_url.trim().is_empty() {
            return Ok(None);
        }
        let mut base = reqwest::Url::parse(&config.test_rail_base_url)
            .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        base.set_query(None);
        base.set_fragment(None);
        let client =
            crate::external_http::client(Duration::from_secs(config.test_rail_timeout_seconds))
                .map_err(|error| TestCatalogError::Request(error.to_string()))?;
        Ok(Some(Arc::new(Self {
            config,
            client,
            base,
        })))
    }

    /// PURPOSE: Encode TestRail's odd `index.php?/api/...` query-as-path style.
    /// USE: All authenticated GET/POST helpers build URLs through this.
    fn api_url(&self, endpoint: &str, query: &[(&str, &str)]) -> reqwest::Url {
        let mut encoded_query = self.base.clone();
        encoded_query.set_query(None);
        encoded_query
            .query_pairs_mut()
            .extend_pairs(query.iter().copied());

        let mut route = format!("/{}", endpoint.trim_start_matches('/'));
        if let Some(query) = encoded_query.query() {
            route.push('&');
            route.push_str(query);
        }

        let mut url = self.base.clone();
        url.set_query(Some(&route));
        url
    }

    /// PURPOSE: Normalize absolute or relative `_links.next` into a same-origin URL.
    /// USE: Pagination loops after each page response.
    fn pagination_url(&self, value: &str) -> Result<reqwest::Url, TestCatalogError> {
        if let Ok(url) = reqwest::Url::parse(value) {
            return Ok(url);
        }

        let route = value
            .split_once("index.php?")
            .map_or(value, |(_, route)| route)
            .trim_start_matches(['?', '/'])
            .replacen('?', "&", 1);
        Ok(self.api_url(&route, &[]))
    }

    /// PURPOSE: GET a suites page with basic auth and typed deserialization.
    /// USE: [`TestCatalog::plans`] pagination.
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

    /// PURPOSE: GET an arbitrary TestRail JSON endpoint with basic auth.
    /// USE: Sections, cases, statuses, and other untyped responses.
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

/// Wire shape for paginated `get_suites` responses.
#[derive(Deserialize)]
struct PlanPage {
    #[serde(default)]
    suites: Vec<PlanWire>,
    #[serde(default, rename = "_links")]
    links: Links,
}
/// Minimal suite fields deserialized from TestRail.
#[derive(Deserialize)]
struct PlanWire {
    id: u64,
    name: String,
    description: Option<String>,
}
/// Pagination `_links` object from TestRail list endpoints.
#[derive(Default, Deserialize)]
struct Links {
    next: Option<String>,
}

#[async_trait]
impl TestCatalog for TestRailCatalog {
    /// PURPOSE: List suites-as-plans with optional name/description filter.
    /// USE: Plan picker APIs; paginates until `_links.next` is exhausted.
    async fn plans(&self, filter: Option<&str>) -> Result<Vec<TestPlan>, TestCatalogError> {
        let endpoint = format!(
            "api/{}/get_suites/{}",
            self.config.test_rail_api_version, self.config.test_rail_project_id
        );
        let mut next = Some(self.api_url(&endpoint, &[("limit", "250"), ("offset", "0")]));
        let mut plans = Vec::new();
        while let Some(url) = next.take() {
            // Reject SSRF via absolute pagination links to other hosts.
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
                .map(|value| self.pagination_url(value))
                .transpose()?;
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

    /// PURPOSE: List sections for a TestRail suite (farm plan id).
    /// USE: Suite picker after a plan is chosen.
    async fn suites(&self, plan_id: u64) -> Result<Vec<TestSuite>, TestCatalogError> {
        let endpoint = format!(
            "api/{}/get_sections/{}",
            self.config.test_rail_api_version, self.config.test_rail_project_id
        );
        let plan_id_query = plan_id.to_string();
        let mut next = Some(self.api_url(
            &endpoint,
            &[
                ("limit", "250"),
                ("offset", "0"),
                ("suite_id", &plan_id_query),
            ],
        ));
        let mut suites = Vec::new();
        while let Some(url) = next.take() {
            // Reject SSRF via absolute pagination links to other hosts.
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
                .map(|value| self.pagination_url(value))
                .transpose()?;
        }
        Ok(suites)
    }

    /// PURPOSE: List cases for a suite/section with optional TestRail filter.
    /// USE: Case selection UI and execution expansion.
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
        let plan_id = plan_id.to_string();
        let suite_id = suite_id.to_string();
        let mut query = vec![
            ("limit", "250"),
            ("offset", "0"),
            ("suite_id", plan_id.as_str()),
            ("section_id", suite_id.as_str()),
        ];
        if let Some(filter) = filter {
            query.push(("filter", filter));
        }
        let first = self.api_url(&endpoint, &query);
        let mut next = Some(first);
        let mut cases = Vec::new();
        while let Some(url) = next.take() {
            // Reject SSRF via absolute pagination links to other hosts.
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
                .map(|value| self.pagination_url(value))
                .transpose()?;
        }
        Ok(cases)
    }

    /// PURPOSE: Map farm PASS/FAIL strings to TestRail status ids and post a result.
    /// USE: After a case finishes on a DUT.
    async fn update_result(&self, update: &TestResultUpdate) -> Result<(), TestCatalogError> {
        // Farm result strings → TestRail status names (resolved to ids below).
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
        let status_url = self.api_url(
            &format!("api/{}/get_statuses", self.config.test_rail_api_version),
            &[],
        );
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
        let result_url = self.api_url(
            &format!(
                "api/{}/add_result_for_case/{}/{}",
                self.config.test_rail_api_version, update.run_id, update.case_id
            ),
            &[],
        );
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

    /// PURPOSE: Create a TestRail run for the selected cases and return its id.
    /// USE: Execution start when the farm records results against an external run.
    async fn create_run(&self, request: &TestRunCreation) -> Result<u64, TestCatalogError> {
        let url = self.api_url(
            &format!(
                "api/{}/add_run/{}",
                self.config.test_rail_api_version, self.config.test_rail_project_id
            ),
            &[],
        );
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

    /// PURPOSE: Delete a previously created TestRail run by id.
    /// USE: Cleanup after cancelled or discarded executions that allocated a run.
    async fn delete_run(&self, run_id: u64) -> Result<(), TestCatalogError> {
        let url = self.api_url(
            &format!(
                "api/{}/delete_run/{run_id}",
                self.config.test_rail_api_version
            ),
            &[],
        );
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

    /// PURPOSE: Download a test script from GitLab/local script hosting with size limits.
    /// USE: Artifact/test preparation workers that need the case's script body on disk.
    async fn test_script(&self, path: &str) -> Result<Vec<u8>, TestCatalogError> {
        use futures_util::StreamExt;

        let path = path.trim().trim_start_matches('/');
        // Block path traversal before joining onto local or GitLab bases.
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
        // Stream with a running size check so Content-Length cannot be lied about.
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

/// PURPOSE: Read a u64 field from JSON, defaulting to 0 when missing.
/// USE: Case/section mapping helpers.
fn number(value: &serde_json::Value, field: &str) -> u64 {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default()
}
/// PURPOSE: Read a string field from JSON, defaulting to empty.
/// USE: Case/section mapping helpers.
fn text(value: &serde_json::Value, field: &str) -> String {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
/// PURPOSE: Percent-decode and strip HTML from TestRail custom fields.
/// USE: Normalize `custom_steps` / `custom_preconds` into plain script paths.
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
