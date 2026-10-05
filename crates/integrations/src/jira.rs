#[cfg(test)]
mod tests;

use std::time::Duration;

use serde::Deserialize;

use crate::config::TestsConfig;

#[derive(Debug, thiserror::Error)]
pub enum JiraError {
    #[error("invalid Jira URL: {0}")]
    InvalidUrl(String),
    #[error("Jira request failed: {0}")]
    Request(String),
    #[error("Jira issue response did not include a key")]
    MissingIssueKey,
}

pub struct JiraDefect {
    pub summary: String,
    pub script_file: String,
    pub device_family: String,
    pub build: String,
    pub test_plan: String,
    pub test_suite: String,
    pub execution_id: String,
    pub test_rail_run: String,
    pub comments: String,
}

pub struct JiraClient {
    client: reqwest::Client,
    api_base: reqwest::Url,
    browse_base: reqwest::Url,
    project_key: String,
    script_base_url: String,
}

impl JiraClient {
    pub fn configured(config: &TestsConfig) -> Result<Option<Self>, JiraError> {
        if config.jira_base_url.trim().is_empty() {
            return Ok(None);
        }
        let mut browse_base = reqwest::Url::parse(&config.jira_base_url)
            .map_err(|error| JiraError::InvalidUrl(error.to_string()))?;
        if !browse_base.path().ends_with('/') {
            browse_base.set_path(&format!("{}/", browse_base.path()));
        }
        let endpoint = format!("{}/", config.jira_endpoint.trim_matches('/'));
        let api_base = browse_base
            .join(&endpoint)
            .map_err(|error| JiraError::InvalidUrl(error.to_string()))?;
        let mut authorization =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", config.jira_api_token))
                .map_err(|error| JiraError::Request(error.to_string()))?;
        authorization.set_sensitive(true);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, authorization);
        let client =
            crate::external_http::client_builder(Duration::from_secs(config.jira_timeout_seconds))
                .default_headers(headers)
                .build()
                .map_err(|error| JiraError::Request(error.to_string()))?;
        Ok(Some(Self {
            client,
            api_base,
            browse_base,
            project_key: config.jira_project_key.clone(),
            script_base_url: config.jira_script_base_url.clone(),
        }))
    }

    pub async fn add_defect(&self, defect: &JiraDefect) -> Result<String, JiraError> {
        let jql_summary = defect.summary.replace(['\\', '"'], " ");
        let jql = format!(
            "project = {} AND status IN (\"To Do\", \"In Progress\") AND summary ~ \"{}\"",
            self.project_key, jql_summary
        );
        let existing = self
            .client
            .get(self.endpoint("search")?)
            .query(&[
                ("jql", jql),
                ("fields", "key,summary,status,self".to_owned()),
            ])
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        if let Ok(response) = existing {
            let search = response
                .json::<SearchResponse>()
                .await
                .map_err(|error| JiraError::Request(error.to_string()))?;
            if let Some(issue) = search.issues.first() {
                let response = self
                    .client
                    .post(self.endpoint(&format!("issue/{}/comment", issue.key))?)
                    .json(&serde_json::json!({"body": self.description(defect)}))
                    .send()
                    .await
                    .map_err(|error| JiraError::Request(format!("{error:?}")))?;
                if !response.status().is_success() {
                    tracing::warn!(status = %response.status(), issue = issue.key, "Jira incident comment failed");
                }
                return self.browse_url(&issue.key);
            }
        } else {
            tracing::warn!("Jira issue search failed; attempting issue creation");
        }
        let response = self
            .client
            .post(self.endpoint("issue")?)
            .json(&serde_json::json!({
                "fields": {
                    "project": {"key": self.project_key},
                    "summary": truncate_chars(&defect.summary, 255),
                    "description": self.description(defect),
                    "issuetype": {"name": "Bug"}
                }
            }))
            .send()
            .await
            .map_err(|error| JiraError::Request(format!("{error:?}")))?
            .error_for_status()
            .map_err(|error| JiraError::Request(error.to_string()))?
            .json::<Issue>()
            .await
            .map_err(|error| JiraError::Request(error.to_string()))?;
        if response.key.trim().is_empty() {
            return Err(JiraError::MissingIssueKey);
        }
        self.browse_url(&response.key)
    }

    fn endpoint(&self, path: &str) -> Result<reqwest::Url, JiraError> {
        self.api_base
            .join(path)
            .map_err(|error| JiraError::InvalidUrl(error.to_string()))
    }

    fn browse_url(&self, key: &str) -> Result<String, JiraError> {
        self.browse_base
            .join(&format!("browse/{key}"))
            .map(|url| url.to_string())
            .map_err(|error| JiraError::InvalidUrl(error.to_string()))
    }

    fn description(&self, defect: &JiraDefect) -> String {
        let script_url = if self.script_base_url.trim().is_empty() {
            String::new()
        } else {
            format!(
                "{}/{}",
                self.script_base_url.trim_end_matches('/'),
                defect.script_file.trim_start_matches('/')
            )
        };
        [
            format!("Execution ID: {}", defect.execution_id),
            format!("TestRail Run: {}", defect.test_rail_run),
            format!("Script: {}", defect.script_file),
            format!("Script URL: {script_url}"),
            format!("Device Family: {}", defect.device_family),
            format!("Build: {}", defect.build),
            format!("Test Plan: {}", defect.test_plan),
            format!("Test Suite: {}", defect.test_suite),
            format!("Incident Time: {}", chrono::Utc::now().to_rfc3339()),
            String::new(),
            "Comments:".to_owned(),
            defect.comments.clone(),
        ]
        .join("\n")
    }
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    issues: Vec<Issue>,
}

#[derive(Deserialize)]
struct Issue {
    key: String,
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
