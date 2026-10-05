#[cfg(test)]
pub mod tests;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use async_trait::async_trait;

use crate::config::TestsConfig;

#[derive(Debug, thiserror::Error)]
pub enum QmetryError {
    #[error("Qmetry request failed: {0}")]
    Request(String),
}

#[async_trait]
pub trait QmetryCatalog: Send + Sync {
    async fn cases_by_plan(&self, plan_id: u64) -> Result<serde_json::Value, QmetryError>;
    async fn cases_by_folder(&self, folder_id: u64) -> Result<serde_json::Value, QmetryError>;
}

pub struct HttpQmetryCatalog {
    config: TestsConfig,
    client: reqwest::Client,
    base: reqwest::Url,
}

impl HttpQmetryCatalog {
    pub fn configured(config: TestsConfig) -> Result<Option<Arc<dyn QmetryCatalog>>, QmetryError> {
        if config.qmetry_base_url.trim().is_empty() {
            return Ok(None);
        }
        let base = reqwest::Url::parse(&config.qmetry_base_url)
            .map_err(|error| QmetryError::Request(error.to_string()))?;
        let client =
            crate::external_http::client(Duration::from_secs(config.qmetry_timeout_seconds))
                .map_err(|error| QmetryError::Request(error.to_string()))?;
        Ok(Some(Arc::new(Self {
            config,
            client,
            base,
        })))
    }

    async fn folders(&self) -> Result<serde_json::Value, QmetryError> {
        let url = self
            .base
            .join(&format!(
                "projects/{}/testcase-folders",
                self.config.jira_project_id
            ))
            .map_err(|error| QmetryError::Request(error.to_string()))?;
        self.client
            .get(url)
            .header("apikey", &self.config.qmetry_api_key)
            .query(&[("sort", "NAME:asc"), ("withCount", "true")])
            .send()
            .await
            .map_err(request_error)?
            .error_for_status()
            .map_err(request_error)?
            .json()
            .await
            .map_err(request_error)
    }

    async fn folder_cases(
        &self,
        folder_id: u64,
        labels: bool,
    ) -> Result<Vec<serde_json::Value>, QmetryError> {
        let url = self
            .base
            .join("testcases/search/")
            .map_err(|error| QmetryError::Request(error.to_string()))?;
        let value: serde_json::Value = self.client.post(url).header("apikey", &self.config.qmetry_api_key)
            .query(&[("fields", "seqNo,key,version,summary,priority,status")])
            .json(&serde_json::json!({"filter": {"projectId": self.config.jira_project_id, "folderId": folder_id}}))
            .send().await.map_err(request_error)?.error_for_status().map_err(request_error)?.json().await.map_err(request_error)?;
        Ok(value.get("data").and_then(serde_json::Value::as_array).into_iter().flatten().map(|case| {
            let script = case.get("customFields").and_then(serde_json::Value::as_object).and_then(|fields| fields.values().find(|field| field.get("name").and_then(serde_json::Value::as_str) == Some("Script File"))).and_then(|field| field.get("value")).and_then(serde_json::Value::as_str).unwrap_or_default();
            let mut output = serde_json::json!({"id": case.get("id"), "name": case.get("summary"), "script_file": script, "key": case.get("key"), "seqNo": case.get("seqNo"), "version": case.get("version")});
            if labels { output["labels"] = serde_json::Value::Array(case.get("labels").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|label| label.get("title").and_then(serde_json::Value::as_str).map(|value| serde_json::Value::String(value.to_owned()))).collect()); }
            output
        }).collect())
    }
}

#[async_trait]
impl QmetryCatalog for HttpQmetryCatalog {
    async fn cases_by_plan(&self, plan_id: u64) -> Result<serde_json::Value, QmetryError> {
        let folders = self.folders().await?;
        let children = folders
            .get("data")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .find(|plan| plan.get("id").and_then(serde_json::Value::as_u64) == Some(plan_id))
            .and_then(|plan| plan.get("children"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut result = BTreeMap::new();
        for child in children {
            if let Some(id) = child.get("id").and_then(serde_json::Value::as_u64) {
                let cases = self.folder_cases(id, true).await?;
                if !cases.is_empty() {
                    result.insert(id.to_string(), cases);
                }
            }
        }
        serde_json::to_value(result).map_err(|error| QmetryError::Request(error.to_string()))
    }

    async fn cases_by_folder(&self, folder_id: u64) -> Result<serde_json::Value, QmetryError> {
        Ok(serde_json::Value::Array(
            self.folder_cases(folder_id, false).await?,
        ))
    }
}

fn request_error(error: reqwest::Error) -> QmetryError {
    QmetryError::Request(error.to_string())
}
