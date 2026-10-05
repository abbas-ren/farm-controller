mod performance_graphs;
#[cfg(test)]
pub mod tests;

use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use regex::Regex;
use reqwest::{Client, Url, multipart};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    config::ReportsConfig,
    error::ErrorResponse,
    events::{EventHub, ServerEvent},
    observability::Metrics,
    state::AppState,
};

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportRequest {
    pub test_id: String,
    pub build_version: String,
    pub device_type: String,
    pub previous_test_id: Option<String>,
    pub previous_build_version: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Queued,
    Processing,
    Consolidating,
    GeneratingSanity,
    GeneratingPerf,
    GeneratingGraphs,
    BuildingReport,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportJobSnapshot {
    pub job_id: Uuid,
    pub status: ReportStatus,
    pub test_id: String,
    pub build_version: String,
    pub device_type: String,
    pub error: Option<String>,
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) created_by: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportAcknowledgement {
    pub status: &'static str,
    pub job_id: Uuid,
    pub test_id: String,
    pub build_version: String,
    pub device_type: String,
    pub message: &'static str,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfluenceUploadRequest {
    pub test_id: String,
    pub build_version: String,
    pub device_type: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfluenceUploadResponse {
    pub status: &'static str,
    pub page_id: String,
    pub subpage_title: String,
    pub parent_page_id: String,
    pub uploaded_images: Vec<String>,
    pub failed_images: Vec<String>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct TestCaseRecord {
    pub test_case_id: i64,
    pub suite_name: String,
    pub result: Option<String>,
    pub comment: Option<String>,
    pub command: Option<String>,
    pub jira_defect: Option<String>,
    pub output_file_path: Option<String>,
    pub labels: Option<Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("{0}")]
    Validation(String),
    #[error("report queue is full")]
    QueueFull,
    #[error("report worker is unavailable")]
    WorkerUnavailable,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ConfluenceError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Configuration(String),
    #[error("{0}")]
    Upstream(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[async_trait]
pub trait ReportDataSource: Send + Sync {
    async fn test_cases(&self, test_id: &str) -> Result<Vec<TestCaseRecord>, ReportError>;

    async fn update_report_status(
        &self,
        _report_id: Uuid,
        _status: ReportStatus,
        _error: Option<&str>,
    ) -> Result<(), ReportError> {
        Ok(())
    }
}

type ReportQueueItem = (Uuid, ReportRequest);
type ReportReceiver = mpsc::Receiver<ReportQueueItem>;

pub struct PostgresReportDataSource {
    pool: PgPool,
}

impl PostgresReportDataSource {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReportDataSource for PostgresReportDataSource {
    async fn test_cases(&self, test_id: &str) -> Result<Vec<TestCaseRecord>, ReportError> {
        Ok(sqlx::query_as::<_, TestCaseRecord>(
            r#"
            SELECT "testCaseId" AS test_case_id, "suiteName" AS suite_name,
                   result::text, comment, cmd AS command, "jiraDefect" AS jira_defect,
                   "outputFilePath" AS output_file_path, labels
            FROM testcase
            WHERE "executionId" = $1
            ORDER BY "suiteId", id
            "#,
        )
        .bind(test_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn update_report_status(
        &self,
        report_id: Uuid,
        status: ReportStatus,
        error: Option<&str>,
    ) -> Result<(), ReportError> {
        let status = match status {
            ReportStatus::Completed => "completed",
            ReportStatus::Failed => "failed",
            _ => return Ok(()),
        };
        sqlx::query(
            r#"UPDATE execution_reports
               SET status = $2::"enum_execution_reports_status", "uploadError" = $3,
                   "updatedAt" = now()
               WHERE id = $1"#,
        )
        .bind(report_id)
        .bind(status)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct ReportService {
    config: ReportsConfig,
    jobs: Arc<RwLock<HashMap<Uuid, ReportJobSnapshot>>>,
    sender: mpsc::Sender<ReportQueueItem>,
    receiver: Arc<Mutex<Option<ReportReceiver>>>,
    data_source: Arc<dyn ReportDataSource>,
    metrics: Metrics,
    client: Client,
    event_publisher: Option<Arc<EventHub>>,
}

impl ReportService {
    pub fn new(
        config: ReportsConfig,
        data_source: Arc<dyn ReportDataSource>,
        metrics: Metrics,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(config.queue_capacity);
        let client = crate::external_http::client(std::time::Duration::from_secs(
            config.confluence_timeout_seconds,
        ))
        .expect("static HTTP client configuration is valid");
        Self {
            config,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            sender,
            receiver: Arc::new(Mutex::new(Some(receiver))),
            data_source,
            metrics,
            client,
            event_publisher: None,
        }
    }

    pub(crate) fn with_event_publisher(mut self, event_publisher: Arc<EventHub>) -> Self {
        self.event_publisher = Some(event_publisher);
        self
    }

    pub async fn enqueue(&self, request: ReportRequest) -> Result<Uuid, ReportError> {
        self.enqueue_with_id(Uuid::new_v4(), request).await
    }

    pub async fn enqueue_with_id(
        &self,
        job_id: Uuid,
        request: ReportRequest,
    ) -> Result<Uuid, ReportError> {
        validate_request(&request)?;
        self.jobs.write().await.insert(
            job_id,
            ReportJobSnapshot {
                job_id,
                status: ReportStatus::Queued,
                test_id: request.test_id.clone(),
                build_version: request.build_version.clone(),
                device_type: request.device_type.clone(),
                error: None,
                created_by: request.created_by.clone(),
                updated_at: Utc::now(),
            },
        );
        if let Err(error) = self.sender.try_send((job_id, request)) {
            self.jobs.write().await.remove(&job_id);
            return Err(match error {
                mpsc::error::TrySendError::Full(_) => ReportError::QueueFull,
                mpsc::error::TrySendError::Closed(_) => ReportError::WorkerUnavailable,
            });
        }
        Ok(job_id)
    }

    pub async fn status(&self, job_id: Uuid) -> Option<ReportJobSnapshot> {
        self.jobs.read().await.get(&job_id).cloned()
    }

    pub fn start_worker(
        &self,
        cancellation: CancellationToken,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let service = self.clone();
        Some(tokio::spawn(async move {
            let Some(mut receiver) = service.receiver.lock().await.take() else {
                return;
            };
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    job = receiver.recv() => match job {
                        Some((job_id, request)) => service.process(job_id, request).await,
                        None => break,
                    },
                }
            }
            tracing::info!(worker = "reports", "worker stopped");
        }))
    }

    async fn process(&self, job_id: Uuid, request: ReportRequest) {
        self.set_status(job_id, ReportStatus::Processing, None)
            .await;
        let result = self.generate(job_id, &request).await;
        match result {
            Ok(()) => {
                self.set_status(job_id, ReportStatus::Completed, None).await;
                self.metrics.record_worker_run("reports", "success");
            }
            Err(error) => {
                tracing::error!(%error, %job_id, worker = "reports", "report generation failed");
                self.set_status(job_id, ReportStatus::Failed, Some(error.to_string()))
                    .await;
                self.metrics.record_worker_run("reports", "failure");
            }
        }
    }

    async fn generate(&self, job_id: Uuid, request: &ReportRequest) -> Result<(), ReportError> {
        let root = report_root(&self.config.test_results_dir, request)?;
        tokio::fs::create_dir_all(&root).await?;
        append_log(
            &root,
            "INFO",
            &format!("Report generation started: {job_id}"),
        )
        .await?;
        self.set_status(job_id, ReportStatus::Consolidating, None)
            .await;
        let cases = self.data_source.test_cases(&request.test_id).await?;
        write_suite_artifacts(&root, &cases).await?;
        self.set_status(job_id, ReportStatus::GeneratingSanity, None)
            .await;
        write_sanity_analysis(&root).await?;
        self.set_status(job_id, ReportStatus::GeneratingPerf, None)
            .await;
        if let Err(error) =
            write_performance_comparisons(&self.config.test_results_dir, &root, request).await
        {
            if error.kind() == std::io::ErrorKind::NotFound {
                append_log(
                    &root,
                    "WARNING",
                    &format!("Performance comparison skipped: {error}"),
                )
                .await?;
            } else {
                return Err(error.into());
            }
        }
        self.set_status(job_id, ReportStatus::GeneratingGraphs, None)
            .await;
        if let Err(error) =
            performance_graphs::write(&self.config.test_results_dir, &root, request, &cases).await
        {
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData
            ) {
                append_log(
                    &root,
                    "WARNING",
                    &format!("Performance graphs skipped: {error}"),
                )
                .await?;
            } else {
                return Err(error.into());
            }
        }
        self.set_status(job_id, ReportStatus::BuildingReport, None)
            .await;
        write_combined_report(&root, request, &cases, &self.config.report_base_url).await?;
        append_log(&root, "INFO", "Report generation completed").await?;
        self.metrics
            .record_worker_items("reports", cases.len() as u64);
        Ok(())
    }

    async fn set_status(&self, job_id: Uuid, status: ReportStatus, error: Option<String>) {
        if matches!(status, ReportStatus::Completed | ReportStatus::Failed)
            && let Err(persistence_error) = self
                .data_source
                .update_report_status(job_id, status, error.as_deref())
                .await
        {
            tracing::error!(%persistence_error, %job_id, "failed to persist terminal report status");
            return;
        }
        let snapshot = {
            let mut jobs = self.jobs.write().await;
            jobs.get_mut(&job_id).map(|job| {
                job.status = status;
                job.error = error;
                job.updated_at = Utc::now();
                job.clone()
            })
        };
        if let Some(snapshot) = snapshot {
            self.publish_status(&snapshot);
        }
    }

    fn publish_status(&self, snapshot: &ReportJobSnapshot) {
        let Some(event_publisher) = &self.event_publisher else {
            return;
        };
        let mut payload = serde_json::json!({
            "reportId": snapshot.job_id,
            "testExecutionId": snapshot.test_id,
            "status": snapshot.status,
            "createdBy": snapshot.created_by,
        });
        if let Some(error) = &snapshot.error {
            payload["error"] = error.clone().into();
        }
        let mut rooms = vec![format!("test:{}", snapshot.test_id)];
        if let Some(created_by) = snapshot.created_by.as_deref() {
            rooms.insert(0, format!("user:{created_by}"));
        }
        for room in rooms {
            if let Err(error) = event_publisher.publish(ServerEvent {
                event: "execution_report_update".to_owned(),
                payload: payload.clone(),
                room: Some(room),
            }) {
                tracing::warn!(%error, job_id = %snapshot.job_id, "failed to publish report status");
            }
        }
    }

    pub async fn upload_confluence(
        &self,
        request: ConfluenceUploadRequest,
    ) -> Result<ConfluenceUploadResponse, ConfluenceError> {
        let report_request = ReportRequest {
            test_id: request.test_id.clone(),
            build_version: request.build_version.clone(),
            device_type: request.device_type.clone(),
            previous_test_id: None,
            previous_build_version: None,
            created_by: None,
        };
        let root = report_root(&self.config.test_results_dir, &report_request)
            .map_err(|error| ConfluenceError::Validation(error.to_string()))?;
        let report_path = root.join("report.html");
        if !report_path.is_file() {
            return Err(ConfluenceError::Validation(
                "Report HTML not found. Create the report first before uploading to Confluence."
                    .to_owned(),
            ));
        }
        let settings = ConfluenceSettings::from_config(&self.config, &request.build_version)?;
        let mut report_html = tokio::fs::read_to_string(&report_path).await?;
        report_html = confluence_body(&report_html)?;
        append_log(&root, "INFO", "Confluence upload started").await?;

        let parent = self.lookup_page(&settings, &settings.parent_title).await?;
        let parent_page_id = match parent {
            Some(page) => page.id,
            None => {
                self.create_page(
                    &settings,
                    &settings.parent_title,
                    &settings.top_level_parent_id,
                    &format!("<h1>{}</h1>", escape_html(&settings.parent_title)),
                )
                .await?
            }
        };
        let subpage_title = format!(
            "{} - {} - {} - {}",
            request.build_version, request.device_type, request.test_id, settings.subpage_title
        );
        let page_id = match self.lookup_page(&settings, &subpage_title).await? {
            Some(page) => {
                self.update_page(
                    &settings,
                    &page.id,
                    &subpage_title,
                    &report_html,
                    page.version.number,
                )
                .await?
            }
            None => {
                self.create_page(&settings, &subpage_title, &parent_page_id, &report_html)
                    .await?
            }
        };

        let (uploaded_images, failed_images) =
            self.upload_images(&settings, &page_id, &root).await?;
        append_log(
            &root,
            "INFO",
            &format!("Confluence upload completed: page {page_id}"),
        )
        .await?;
        Ok(ConfluenceUploadResponse {
            status: "uploaded",
            page_id,
            subpage_title,
            parent_page_id,
            uploaded_images,
            failed_images,
        })
    }

    async fn lookup_page(
        &self,
        settings: &ConfluenceSettings,
        title: &str,
    ) -> Result<Option<ConfluencePage>, ConfluenceError> {
        let response = self
            .client
            .get(settings.endpoint("rest/api/content")?)
            .bearer_auth(&settings.pat)
            .query(&[
                ("spaceKey", settings.space.as_str()),
                ("title", title),
                ("expand", "version,ancestors"),
            ])
            .send()
            .await
            .map_err(upstream_transport)?;
        let response = upstream_response(response).await?;
        let body: ConfluenceSearch = response.json().await.map_err(upstream_transport)?;
        Ok(body.results.into_iter().next())
    }

    async fn create_page(
        &self,
        settings: &ConfluenceSettings,
        title: &str,
        parent_id: &str,
        body: &str,
    ) -> Result<String, ConfluenceError> {
        let response = self
            .client
            .post(settings.endpoint("rest/api/content")?)
            .bearer_auth(&settings.pat)
            .json(&serde_json::json!({
                "type": "page", "title": title, "ancestors": [{"id": parent_id}],
                "space": {"key": settings.space},
                "body": {"storage": {"value": body, "representation": "storage"}}
            }))
            .send()
            .await
            .map_err(upstream_transport)?;
        let response = upstream_response(response).await?;
        let page: ConfluencePageId = response.json().await.map_err(upstream_transport)?;
        Ok(page.id)
    }

    async fn update_page(
        &self,
        settings: &ConfluenceSettings,
        page_id: &str,
        title: &str,
        body: &str,
        current_version: u64,
    ) -> Result<String, ConfluenceError> {
        let response = self
            .client
            .put(settings.endpoint(&format!("rest/api/content/{page_id}"))?)
            .bearer_auth(&settings.pat)
            .json(&serde_json::json!({
                "type": "page", "title": title, "version": {"number": current_version + 1},
                "body": {"storage": {"value": body, "representation": "storage"}}
            }))
            .send()
            .await
            .map_err(upstream_transport)?;
        let response = upstream_response(response).await?;
        let page: ConfluencePageId = response.json().await.map_err(upstream_transport)?;
        Ok(page.id)
    }

    async fn upload_images(
        &self,
        settings: &ConfluenceSettings,
        page_id: &str,
        root: &Path,
    ) -> Result<(Vec<String>, Vec<String>), ConfluenceError> {
        let directory = root.join("perf_compare");
        let mut names = vec!["radar_chart.png".to_owned()];
        names.extend(
            ["network", "storage", "cpu", "memory", "dma", "gpu"]
                .map(|category| format!("{category}_bar.png")),
        );
        if let Ok(mut entries) = tokio::fs::read_dir(&directory).await {
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("test_") && name.ends_with("_comp.png") {
                    names.push(name);
                }
            }
        }
        names.sort();
        names.dedup();
        let mut uploaded = Vec::new();
        let mut failed = Vec::new();
        for name in names {
            let path = directory.join(&name);
            if !path.is_file() {
                failed.push(name);
                continue;
            }
            let bytes = tokio::fs::read(&path).await?;
            let form = multipart::Form::new().part(
                "file",
                multipart::Part::bytes(bytes)
                    .file_name(name.clone())
                    .mime_str("image/png")
                    .map_err(|error| ConfluenceError::Upstream(error.to_string()))?,
            );
            let request = self
                .client
                .post(settings.endpoint(&format!("rest/api/content/{page_id}/child/attachment"))?)
                .bearer_auth(&settings.pat)
                .header("X-Atlassian-Token", "no-check")
                .multipart(form);
            match request.send().await {
                Ok(response) if response.status().is_success() => uploaded.push(name),
                Ok(response) => failed.push(format!("{} (HTTP {})", name, response.status())),
                Err(error) => failed.push(format!("{name} ({error})")),
            }
        }
        Ok((uploaded, failed))
    }
}

#[derive(Debug)]
struct ConfluenceSettings {
    api_base: Url,
    pat: String,
    space: String,
    parent_title: String,
    subpage_title: String,
    top_level_parent_id: String,
}

impl ConfluenceSettings {
    fn from_config(config: &ReportsConfig, build_version: &str) -> Result<Self, ConfluenceError> {
        let required = [
            ("confluence_url", config.confluence_url.trim()),
            ("confluence_pat", config.confluence_pat.trim()),
            ("confluence_space", config.confluence_space.trim()),
            (
                "confluence_subpage_title",
                config.confluence_subpage_title.trim(),
            ),
        ];
        let missing: Vec<_> = required
            .iter()
            .filter_map(|(name, value)| value.is_empty().then_some(*name))
            .collect();
        if !missing.is_empty() {
            return Err(ConfluenceError::Configuration(format!(
                "Missing Confluence configuration: {}",
                missing.join(", ")
            )));
        }
        let source = Url::parse(config.confluence_url.trim()).map_err(|error| {
            ConfluenceError::Configuration(format!("Invalid Confluence URL: {error}"))
        })?;
        let segments: Vec<_> = source.path_segments().into_iter().flatten().collect();
        let top_level_parent_id = segments
            .windows(2)
            .find_map(|parts| {
                (parts[0] == "pages"
                    && parts[1].chars().all(|character| character.is_ascii_digit()))
                .then(|| parts[1].to_owned())
            })
            .ok_or_else(|| {
                ConfluenceError::Validation(
                    "Could not extract parent page ID from Confluence URL".to_owned(),
                )
            })?;
        let mut api_base = source;
        api_base.set_path("/");
        api_base.set_query(None);
        api_base.set_fragment(None);
        Ok(Self {
            api_base,
            pat: config.confluence_pat.clone(),
            space: config.confluence_space.clone(),
            parent_title: if config.confluence_parent_title.trim().is_empty() {
                build_version.to_owned()
            } else {
                config.confluence_parent_title.clone()
            },
            subpage_title: config.confluence_subpage_title.clone(),
            top_level_parent_id,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url, ConfluenceError> {
        self.api_base
            .join(path)
            .map_err(|error| ConfluenceError::Configuration(error.to_string()))
    }
}

#[derive(Deserialize)]
struct ConfluenceSearch {
    results: Vec<ConfluencePage>,
}

#[derive(Deserialize)]
struct ConfluencePage {
    id: String,
    version: ConfluenceVersion,
}

#[derive(Deserialize)]
struct ConfluenceVersion {
    number: u64,
}

#[derive(Deserialize)]
struct ConfluencePageId {
    id: String,
}

fn upstream_transport(error: reqwest::Error) -> ConfluenceError {
    ConfluenceError::Upstream(error.to_string())
}

async fn upstream_response(
    response: reqwest::Response,
) -> Result<reqwest::Response, ConfluenceError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(ConfluenceError::Upstream(format!(
        "Confluence returned HTTP {status}: {}",
        body.chars().take(1000).collect::<String>()
    )))
}

fn confluence_body(document: &str) -> Result<String, ConfluenceError> {
    let body = Regex::new(r"(?is)<body[^>]*>(.*)</body>")
        .expect("static regex is valid")
        .captures(document)
        .and_then(|captures| captures.get(1))
        .map_or(document, |body| body.as_str());
    let images =
        Regex::new(r#"(?i)<img\b[^>]*src="([^"]*\.png)"[^>]*>"#).expect("static regex is valid");
    Ok(images
        .replace_all(body, |captures: &regex::Captures<'_>| {
            let filename = Path::new(&captures[1])
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&captures[1]);
            format!(
                r#"<ac:image ac:width="600"><ri:attachment ri:filename="{}"/></ac:image>"#,
                escape_html(filename)
            )
        })
        .into_owned())
}

fn validate_request(request: &ReportRequest) -> Result<(), ReportError> {
    for (name, value) in [
        ("testId", request.test_id.as_str()),
        ("buildVersion", request.build_version.as_str()),
        ("deviceType", request.device_type.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(ReportError::Validation(format!(
                "{name} is required and cannot be empty"
            )));
        }
        let mut components = Path::new(value).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err(ReportError::Validation(format!(
                "invalid path component: {name}"
            )));
        }
    }
    Ok(())
}

pub(super) fn report_root(base: &Path, request: &ReportRequest) -> Result<PathBuf, ReportError> {
    validate_request(request)?;
    Ok(base
        .join(request.device_type.trim())
        .join(request.build_version.trim())
        .join(request.test_id.trim()))
}

async fn append_log(root: &Path, level: &str, message: &str) -> Result<(), std::io::Error> {
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("report_generation.log"))
        .await?;
    file.write_all(format!("[{}] [{}] {}\n", Utc::now().to_rfc3339(), level, message).as_bytes())
        .await
}

async fn write_suite_artifacts(
    root: &Path,
    cases: &[TestCaseRecord],
) -> Result<(), std::io::Error> {
    for (suite, text_name, html_name) in [
        (
            "sanity",
            "sanity_commands_output.txt",
            "sanity_commands_report.html",
        ),
        (
            "performance",
            "performance_commands_output.txt",
            "performance_commands_report.html",
        ),
    ] {
        let selected: Vec<_> = cases
            .iter()
            .filter(|case| case.suite_name.eq_ignore_ascii_case(suite))
            .collect();
        if selected.is_empty() {
            continue;
        }
        let mut text = String::new();
        let mut rows = String::new();
        for case in selected {
            let result = case.result.as_deref().unwrap_or("FAIL");
            let command = case.command.as_deref().unwrap_or("N/A");
            let output_path = case.output_file_path.as_deref().unwrap_or("");
            let output_name = file_name(case.output_file_path.as_deref(), case.test_case_id);
            if suite == "performance" {
                let module = case
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.first())
                    .map_or("", String::as_str);
                text.push_str(&format!(
                    "Test {} - Module: {module}\nCommand: {command}\nSaved to: {output_name}\n\n{}\n",
                    case.test_case_id,
                    "-".repeat(60)
                ));
                rows.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>View Output</a></td><td></td><td>{}</td><td>{}</td></tr>",
                    case.test_case_id,
                    escape_html(module),
                    escape_html(command),
                    escape_html(output_path),
                    jira_cell(case.jira_defect.as_deref()),
                    escape_html(case.comment.as_deref().unwrap_or(""))
                ));
            } else {
                text.push_str(&format!(
                    "Test {}\nCommand: {command}\nSaved to: {output_name}\n\n{}\n",
                    case.test_case_id,
                    "-".repeat(60)
                ));
                let color = if result == "PASS" { "green" } else { "red" };
                rows.push_str(&format!(
                    "<tr style='color:{color};'><td>{}</td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>View Output</a></td><td>{}</td><td>{}</td></tr>",
                    case.test_case_id,
                    escape_html(command),
                    escape_html(result),
                    escape_html(output_path),
                    jira_cell(case.jira_defect.as_deref()),
                    escape_html(case.comment.as_deref().unwrap_or(""))
                ));
            }
        }
        let (title, header) = if suite == "performance" {
            (
                "Performance Benchmarking",
                "<tr><th>Test No</th><th>Module</th><th>Command</th><th>Measured Result</th><th>Expected Result</th><th>JIRA ID</th><th>Remark</th></tr>",
            )
        } else {
            (
                "Sanity",
                "<tr><th>Test No</th><th>Commands</th><th>Test Results</th><th>Logs</th><th>JIRA ID</th><th>Remark</th></tr>",
            )
        };
        tokio::fs::write(root.join(text_name), text).await?;
        tokio::fs::write(root.join(html_name), format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title} Test Report:</title></head><body><h2>{title} Test Report</h2><table border='1' cellpadding='5' cellspacing='0'><thead>{header}</thead><tbody>{rows}</tbody></table></body></html>"
        )).await?;
    }
    Ok(())
}

async fn write_sanity_analysis(root: &Path) -> Result<(), std::io::Error> {
    let results = root.join("sanity_commands_results");
    if !results.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Sanity results directory not found: {}", results.display()),
        ));
    }
    for required in [
        root.join("sanity_commands_report.html"),
        root.join("sanity_commands_output.txt"),
    ] {
        if !required.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Sanity report artifact not found: {}", required.display()),
            ));
        }
    }

    let mut entries = tokio::fs::read_dir(&results).await?;
    let mut inputs = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        {
            inputs.push(path);
        }
    }
    inputs.sort();
    if inputs.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "No .txt files found in sanity results: {}",
                results.display()
            ),
        ));
    }

    let output = root.join("sanity_command_analysis");
    tokio::fs::create_dir_all(&output).await?;
    for input in inputs {
        let content = tokio::fs::read_to_string(&input).await?;
        let file_name = input
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sanity.txt");
        let lines = content.lines().collect::<Vec<_>>();
        let errors = lines
            .iter()
            .filter(|line| line.to_ascii_lowercase().contains("error"))
            .take(5)
            .copied()
            .collect::<Vec<_>>();
        let warnings = lines
            .iter()
            .filter(|line| line.to_ascii_lowercase().contains("warn"))
            .take(5)
            .copied()
            .collect::<Vec<_>>();
        let list = |items: &[&str]| {
            items
                .iter()
                .map(|item| format!("<li>{}</li>", escape_html(item)))
                .collect::<String>()
        };
        let html = format!(
            "<!doctype html>\n<html>\n<head>\n  <meta charset=\"utf-8\" />\n  <title>Analysis Report</title>\n  <style>\n    body {{ font-family: Arial, sans-serif; margin: 20px; }}\n    h1, h2 {{ color: #2c3e50; }}\n    ul {{ padding-left: 20px; }}\n    pre {{ background: #f4f4f4; padding: 10px; border: 1px solid #ccc; white-space: pre-wrap; }}\n  </style>\n</head>\n<body>\n  <h1>Analysis Report: {}</h1>\n  <p>Total lines: {} | Errors: {} | Warnings: {}</p>\n  <h2>Top Errors</h2>\n  <ul>{}</ul>\n  <h2>Top Warnings</h2>\n  <ul>{}</ul>\n  <h2>Raw Output</h2>\n  <pre>{}</pre>\n</body>\n</html>\n",
            escape_html(file_name),
            lines.len(),
            errors.len(),
            warnings.len(),
            list(&errors),
            list(&warnings),
            escape_html(&content),
        );
        let stem = input
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("sanity");
        tokio::fs::write(output.join(format!("{stem}_analysis.html")), html).await?;
    }
    Ok(())
}

async fn write_performance_comparisons(
    results_root: &Path,
    current_root: &Path,
    request: &ReportRequest,
) -> Result<(), std::io::Error> {
    let current = current_root.join("performance_commands_results");
    if !current.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "Performance results directory not found: {}",
                current.display()
            ),
        ));
    }
    let previous_request = ReportRequest {
        test_id: request
            .previous_test_id
            .clone()
            .unwrap_or_else(|| request.test_id.clone()),
        build_version: request
            .previous_build_version
            .clone()
            .unwrap_or_else(|| request.build_version.clone()),
        device_type: request.device_type.clone(),
        previous_test_id: None,
        previous_build_version: None,
        created_by: None,
    };
    let previous_root = report_root(results_root, &previous_request)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let previous = previous_root.join("performance_commands_results");

    let mut entries = tokio::fs::read_dir(&current).await?;
    let mut inputs = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.is_file()
            && name.ends_with(".txt")
            && !name.to_ascii_lowercase().contains("cleanup")
        {
            inputs.push(path);
        }
    }
    inputs.sort();
    if inputs.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("No .txt performance files found in: {}", current.display()),
        ));
    }

    let output = current_root.join("perf_compare");
    tokio::fs::create_dir_all(&output).await?;
    let old_label = format!(
        "{}/{}",
        previous_request.build_version, previous_request.test_id
    );
    let new_label = format!("{}/{}", request.build_version, request.test_id);
    for input in inputs {
        let file_name = input
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("performance.txt");
        let current_content = tokio::fs::read_to_string(&input).await?;
        let (new_metrics, new_command) = performance_metrics(&current_content);
        let previous_path = previous.join(file_name);
        let (old_metrics, old_command) = if previous_path.is_file() {
            performance_metrics(&tokio::fs::read_to_string(previous_path).await?)
        } else {
            (new_metrics.clone(), new_command.clone())
        };
        let command = if old_command == new_command {
            new_command
        } else {
            format!("{old_command} vs {new_command}")
        };
        let mut keys = old_metrics
            .keys()
            .filter(|key| new_metrics.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        let base = file_name
            .strip_suffix("_result.txt")
            .or_else(|| file_name.strip_suffix(".txt"))
            .unwrap_or(file_name);
        let chart_name = format!("{base}_comp.png");
        if !keys.is_empty() {
            let chart_path = output.join(&chart_name);
            let chart_keys = keys.clone();
            let chart_old = chart_keys
                .iter()
                .map(|key| old_metrics[key])
                .collect::<Vec<_>>();
            let chart_new = chart_keys
                .iter()
                .map(|key| new_metrics[key])
                .collect::<Vec<_>>();
            let chart_old_label = old_label.clone();
            let chart_new_label = new_label.clone();
            tokio::task::spawn_blocking(move || {
                render_performance_comparison(
                    &chart_path,
                    &chart_keys,
                    &chart_old,
                    &chart_new,
                    &chart_old_label,
                    &chart_new_label,
                )
            })
            .await
            .map_err(std::io::Error::other)??;
        }
        let html = if keys.is_empty() {
            format!(
                "<!doctype html>\n<html><head><title>{base}_comp Skipped</title></head>\n<body>\n  <h2>&#9888; Skipped: {}</h2>\n  <p>No common performance metrics between\n     <strong>{}</strong> and <strong>{}</strong>.</p>\n  <p><strong>Command:</strong> {}</p>\n</body></html>",
                escape_html(file_name),
                escape_html(&old_label),
                escape_html(&new_label),
                escape_html(&command),
            )
        } else {
            let rows = keys
                .iter()
                .map(|key| {
                    format!(
                        "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                        escape_html(key),
                        old_metrics[key],
                        new_metrics[key]
                    )
                })
                .collect::<String>();
            format!(
                "<!doctype html>\n<html>\n<head>\n  <meta charset=\"utf-8\" />\n  <title>{base}_comp Comparison</title>\n  <style>\n    body {{ font-family: Arial, sans-serif; margin: 20px; }}\n    h2, h3 {{ color: #2c3e50; }}\n    table {{ border-collapse: collapse; }}\n    th, td {{ border: 1px solid #ccc; padding: 6px 10px; }}\n    th {{ background: #f4f4f4; }}\n  </style>\n</head>\n<body>\n  <h2>Performance Comparison &mdash; {}</h2>\n  <p><strong>Previous:</strong> {} &nbsp;&nbsp;\n     <strong>Current:</strong> {}</p>\n  <p><img src=\"{}\" alt=\"{}\" style=\"max-width:1000px;\"/></p>\n  <h3>Detailed Metrics</h3>\n  <table>\n    <tr><th>Metric</th><th>{}</th><th>{}</th></tr>\n    {rows}\n  </table>\n</body>\n</html>",
                escape_html(&command),
                escape_html(&old_label),
                escape_html(&new_label),
                escape_html(&chart_name),
                escape_html(&chart_name),
                escape_html(&old_label),
                escape_html(&new_label),
            )
        };
        tokio::fs::write(output.join(format!("{base}_comp.html")), html).await?;
    }
    Ok(())
}

fn render_performance_comparison(
    path: &Path,
    labels: &[String],
    old_values: &[f64],
    new_values: &[f64],
    old_label: &str,
    new_label: &str,
) -> Result<(), std::io::Error> {
    use plotters::prelude::*;

    let path = path.to_string_lossy().into_owned();
    let root = BitMapBackend::new(&path, (1200, 650)).into_drawing_area();
    root.fill(&WHITE)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let maximum = old_values
        .iter()
        .chain(new_values)
        .copied()
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let slots = labels.len() * 2;
    let mut chart = ChartBuilder::on(&root)
        .caption(
            format!("Performance Comparison - {old_label} vs {new_label}"),
            ("sans-serif", 28),
        )
        .margin(20)
        .x_label_area_size(90)
        .y_label_area_size(80)
        .build_cartesian_2d(0..slots, 0.0..maximum * 1.15)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    chart
        .configure_mesh()
        .x_labels(labels.len())
        .x_label_formatter(&|slot| labels.get(*slot / 2).cloned().unwrap_or_default())
        .draw()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    chart
        .draw_series(old_values.iter().enumerate().map(|(index, value)| {
            Rectangle::new([(index * 2, 0.0), (index * 2 + 1, *value)], BLUE.filled())
        }))
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    chart
        .draw_series(new_values.iter().enumerate().map(|(index, value)| {
            Rectangle::new(
                [(index * 2 + 1, 0.0), (index * 2 + 2, *value)],
                RED.filled(),
            )
        }))
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    root.present()
        .map_err(|error| std::io::Error::other(error.to_string()))
}

pub(super) fn performance_metrics(content: &str) -> (HashMap<String, f64>, String) {
    let lines = content.lines().collect::<Vec<_>>();
    let mut metrics = HashMap::new();
    let mut command = "Unknown".to_owned();
    for line in &lines {
        if let Some(value) = line.strip_prefix("Command:") {
            command = value.trim().to_owned();
        } else if let Some((key, value)) = line.split_once(':')
            && let Ok(value) = value.trim().parse::<f64>()
        {
            metrics.insert(key.trim().to_owned(), value);
        }
    }
    if let Some(total) = lines.iter().find(|line| line.trim().starts_with("Total")) {
        let values = total.split('|').map(str::trim).collect::<Vec<_>>();
        if let Some(value) = values.get(1).and_then(|value| value.parse::<f64>().ok()) {
            metrics.insert("Total_DMIPS".to_owned(), value);
        }
        if values.len() == 3
            && let Ok(value) = values[2].parse::<f64>()
        {
            metrics.insert("Total_DMIPS_per_MHz".to_owned(), value);
        }
    }
    if let Some(value) = lines.iter().rev().find_map(|line| {
        let tokens = line.split_whitespace().collect::<Vec<_>>();
        (tokens.len() >= 5)
            .then(|| tokens.last()?.parse::<f64>().ok())
            .flatten()
    }) {
        metrics.insert("Throughput".to_owned(), value);
    }
    let speed = Regex::new(r"\(([\d.]+) MiB/sec\)").expect("static regex is valid");
    if let Some(value) = lines.iter().rev().find_map(|line| {
        speed
            .captures(line)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse::<f64>().ok())
    }) {
        metrics.insert("Write_Speed_MiB_per_sec".to_owned(), value);
    }
    (metrics, command)
}

async fn write_combined_report(
    root: &Path,
    request: &ReportRequest,
    cases: &[TestCaseRecord],
    report_base_url: &str,
) -> Result<(), std::io::Error> {
    let sanity = cases
        .iter()
        .filter(|case| case.suite_name.eq_ignore_ascii_case("sanity"))
        .collect::<Vec<_>>();
    let passed = sanity
        .iter()
        .filter(|case| case.result.as_deref() == Some("PASS"))
        .count();
    let failed = sanity
        .iter()
        .filter(|case| case.result.as_deref() == Some("FAIL"))
        .count();
    let missing = sanity.len().saturating_sub(passed + failed);
    let pass_percentage = if sanity.is_empty() {
        0.0
    } else {
        passed as f64 / sanity.len() as f64 * 100.0
    };
    let pass_color = if pass_percentage >= 50.0 {
        "green"
    } else {
        "red"
    };
    let report_url = (!report_base_url.trim().is_empty()).then(|| {
        format!(
            "{}/{}/{}/{}",
            report_base_url.trim_end_matches('/'),
            request.device_type,
            request.build_version,
            request.test_id
        )
    });
    let boot_time = if root.join("boottime.html").is_file() {
        parse_boottime_total(&tokio::fs::read_to_string(root.join("boottime.html")).await?)
            .map(|value| format!("{value:.3}"))
            .unwrap_or_default()
    } else {
        String::new()
    };

    let performance_rows = cases
        .iter()
        .filter(|case| case.suite_name.eq_ignore_ascii_case("performance"))
        .map(|case| {
            let output_name = file_name(case.output_file_path.as_deref(), case.test_case_id);
            let output_stem = output_name
                .strip_suffix("_result.txt")
                .or_else(|| output_name.strip_suffix(".txt"))
                .unwrap_or(&output_name);
            let comparison_name = format!("{output_stem}_comp.html");
            let result_href = report_artifact_href(
                report_url.as_deref(),
                &format!("performance_commands_results/{output_name}"),
            );
            let comparison_href = report_artifact_href(
                report_url.as_deref(),
                &format!("perf_compare/{comparison_name}"),
            );
            let module = case
                .labels
                .as_ref()
                .and_then(|labels| labels.first())
                .map_or("", String::as_str);
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>{}</a></td><td></td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>{}</a></td></tr>",
                case.test_case_id,
                escape_html(module),
                escape_html(case.command.as_deref().unwrap_or("")),
                escape_html(&result_href),
                escape_html(&output_name),
                jira_cell(case.jira_defect.as_deref()),
                escape_html(case.comment.as_deref().unwrap_or("")),
                escape_html(&comparison_href),
                escape_html(&comparison_name),
            )
        })
        .collect::<String>();
    let sanity_rows = sanity
        .iter()
        .map(|case| {
            let output_name = file_name(case.output_file_path.as_deref(), case.test_case_id);
            let output_stem = output_name.strip_suffix(".txt").unwrap_or(&output_name);
            let analysis_name = format!("{output_stem}_analysis.html");
            let result_href = report_artifact_href(
                report_url.as_deref(),
                &format!("sanity_commands_results/{output_name}"),
            );
            let analysis_href = report_artifact_href(
                report_url.as_deref(),
                &format!("sanity_command_analysis/{analysis_name}"),
            );
            let result = case.result.as_deref().unwrap_or("FAIL");
            let color = if result == "PASS" { "green" } else { "red" };
            format!(
                "<tr style='color:{color};'><td>{}</td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>{}</a></td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>{}</a></td></tr>",
                case.test_case_id,
                escape_html(case.command.as_deref().unwrap_or("")),
                escape_html(result),
                escape_html(&result_href),
                escape_html(&output_name),
                jira_cell(case.jira_defect.as_deref()),
                escape_html(case.comment.as_deref().unwrap_or("")),
                escape_html(&analysis_href),
                escape_html(&analysis_name),
            )
        })
        .collect::<String>();
    let comparison_dir = root.join("perf_compare");
    let radar_section = if tokio::fs::try_exists(comparison_dir.join("radar_chart.png"))
        .await
        .unwrap_or(false)
    {
        let source = report_url.as_ref().map_or_else(
            || "perf_compare/radar_chart.png".to_owned(),
            |url| format!("{url}/perf_compare/radar_chart.png"),
        );
        format!(
            "<h2>Overall Performance Compared to the Last Release:</h2><div><img src=\"{}\" alt='radar_chart.png' data-ac-width='1000' style='max-width:1000px;'/></div>",
            escape_html(&source)
        )
    } else {
        String::new()
    };
    let mut bar_images = String::new();
    for category in ["network", "storage", "cpu", "memory", "dma", "gpu"] {
        let name = format!("{category}_bar.png");
        if tokio::fs::try_exists(comparison_dir.join(&name))
            .await
            .unwrap_or(false)
        {
            let source = report_url.as_ref().map_or_else(
                || format!("perf_compare/{name}"),
                |url| format!("{url}/perf_compare/{name}"),
            );
            bar_images.push_str(&format!(
                "<td style='padding:15px;border:2px solid #333;'><img src=\"{}\" alt='{name}' data-ac-width='275' style='max-width:275px;'/></td>",
                escape_html(&source)
            ));
        }
    }
    let bars_section = if bar_images.is_empty() {
        String::new()
    } else {
        format!(
            "<h2>Subsystem Performance Comparison:</h2><table style='border-collapse:separate;border-spacing:15px;'><tr>{bar_images}</tr></table>"
        )
    };
    let boot_report_link = report_url.as_ref().map_or_else(
        || "<ac:link><ri:attachment ri:filename='boottime_report.html'/><ac:plain-text-link-body><![CDATA[Click here for Full Boot-Time Report (boottime_report.html)]]></ac:plain-text-link-body></ac:link>".to_owned(),
        |url| format!("<a href=\"{}/boottime_report.html\" target=\"_blank\">Click here for Full Boot-Time Report (boottime_report.html)</a>", escape_html(url)),
    );
    let other_reports = report_url.as_ref().map_or_else(String::new, |url| {
        let rows = [
            ("Busmoni Dashboard", format!("{url}/busmoni_plots/busmoni_dashboard.html")),
            ("PVRTune", format!("{url}/PVRTune_plots/pvrtune_gpu_dashboard.html")),
            ("Test Data", format!("{url}/")),
            ("GitLab Repo Link", "https://rcar-env.dgn.renesas.com/gitlab/rcar-reference-sw/utils/jenkins/".to_owned()),
        ]
        .into_iter()
        .map(|(label, link)| format!("<tr><td style='padding:8px;white-space:nowrap;'>{}</td><td style='padding:8px;white-space:nowrap;'><a href=\"{}\" target=\"_blank\">&#8594; Click here</a></td></tr>", escape_html(label), escape_html(&link)))
        .collect::<String>();
        format!("<hr/><h2>Other Reports:</h2><table border='1' style='border-collapse:collapse;width:auto;table-layout:auto;'><tr><th>Items</th><th>Links</th></tr>{rows}</table><hr/><h2>References:</h2><p><a href=\"https://confluence.renesas.com/spaces/RD2022/pages/315958920/PVRTune+on+x5h\" target=\"_blank\">&#8594; Click here to analyse PVRTune data using GUI, refer this writeup (PVRTune on x5h)</a></p>")
    });

    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{} {} report</title></head><body><h1>{} {}</h1><p>Test ID: {}</p>
<h2>Summary:</h2><table border='1' style='border-collapse:collapse;text-align:center;'><tr><th>SI.No</th><th>Category</th><th>Total Tests</th><th>PASS</th><th>FAIL</th><th>MISSING</th><th>PASS%</th></tr><tr><td>1</td><td>Sanity</td><td>{}</td><td style='color:green'>{passed}</td><td style='color:red'>{failed}</td><td style='color:grey'>{missing}</td><td style='color:{pass_color};font-weight:bold'>{pass_percentage:.1}%</td></tr></table><hr/>
<h2>Boot Time Summary:</h2><table border='1' style='border-collapse:collapse;width:auto;'><tr><th>Stage</th><th>Time (s)</th><th>Reports</th></tr><tr><td>Boot time to root login</td><td>{}</td><td>{boot_report_link}<br/><ac:link><ri:attachment ri:filename='uart.txt'/><ac:plain-text-link-body><![CDATA[Click here to Download UART Log (uart.txt)]]></ac:plain-text-link-body></ac:link></td></tr><tr><td>Boot time to display (Manual)</td><td></td><td></td></tr></table><hr/>{radar_section}{bars_section}
<h2>Performance Benchmarking Test Report</h2><table border='1' cellpadding='5' cellspacing='0'><tr><th>Test No</th><th>Module</th><th>Command</th><th>Measured Result</th><th>Expected Result</th><th>JIRA ID</th><th>Remark</th><th>Comparison With Prev SDK</th></tr>{performance_rows}</table>
<h2>Sanity Test Report</h2><table border='1' cellpadding='5' cellspacing='0'><tr><th>Test No</th><th>Commands</th><th>Test Results</th><th>Logs</th><th>JIRA ID</th><th>Remark</th><th>Analysis</th></tr>{sanity_rows}</table>{other_reports}</body></html>",
        escape_html(&request.device_type),
        escape_html(&request.build_version),
        escape_html(&request.device_type),
        escape_html(&request.build_version),
        escape_html(&request.test_id),
        sanity.len(),
        escape_html(&boot_time),
    );
    tokio::fs::write(root.join("report.html"), html).await
}

fn report_artifact_href(report_url: Option<&str>, relative_path: &str) -> String {
    report_url.map_or_else(
        || relative_path.to_owned(),
        |url| format!("{}/{relative_path}", url.trim_end_matches('/')),
    )
}

fn parse_boottime_total(content: &str) -> Option<f64> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(content);
    let table_selector = Selector::parse("table").expect("static selector");
    let header_selector = Selector::parse("th").expect("static selector");
    let row_selector = Selector::parse("tr").expect("static selector");
    let cell_selector = Selector::parse("td").expect("static selector");
    for table in document.select(&table_selector) {
        let headers = table
            .select(&header_selector)
            .map(|header| header.text().collect::<String>().to_ascii_lowercase())
            .collect::<Vec<_>>();
        if !headers.iter().any(|header| header.contains("stage"))
            || !headers.iter().any(|header| header.contains("time"))
        {
            continue;
        }
        for row in table.select(&row_selector) {
            let cells = row
                .select(&cell_selector)
                .map(|cell| cell.text().collect::<String>().trim().to_owned())
                .collect::<Vec<_>>();
            if cells.len() >= 2 && cells[0].eq_ignore_ascii_case("total") {
                return parse_first_number(&cells[1]);
            }
        }
    }
    let text = document.root_element().text().collect::<Vec<_>>().join(" ");
    Regex::new(r"(?i)\bTotal\b[^0-9]{0,20}([0-9]+(?:\.[0-9]+)?)\b")
        .expect("static regex")
        .captures(&text)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
}

fn parse_first_number(value: &str) -> Option<f64> {
    Regex::new(r"([0-9]+(?:\.[0-9]+)?)")
        .expect("static regex")
        .captures(value)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
}

pub(super) fn file_name(path: Option<&str>, test_case_id: i64) -> String {
    path.and_then(|value| Path::new(value).file_name())
        .and_then(|value| value.to_str())
        .map_or_else(|| format!("test_{test_case_id}_result.txt"), str::to_owned)
}

fn jira_cell(value: Option<&str>) -> String {
    let value = value.unwrap_or("").trim();
    if value.starts_with("http://") || value.starts_with("https://") {
        let escaped = escape_html(value);
        format!("<a href='{escaped}' target='_blank'>{escaped}</a>")
    } else {
        escape_html(value)
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/report", post(enqueue_report))
        .route("/report/{job_id}", get(report_status))
        .route("/upload-confluence", post(upload_confluence))
}

#[utoipa::path(post, path = "/report", tag = "Reports", summary = "Queue report generation", description = "Validates report identifiers and enqueues native generation on the bounded in-process worker.", request_body = ReportRequest,
    responses((status = 202, description = "Report queued", body = ReportAcknowledgement), (status = 400, description = "Invalid request", body = ErrorResponse), (status = 500, description = "Report initialization failed", body = ErrorResponse), (status = 503, description = "Reports or queue unavailable", body = ErrorResponse)))]
pub async fn enqueue_report(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ReportRequest>,
) -> axum::response::Response {
    let Some(service) = &state.reports else {
        return report_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            "Reports are unavailable",
        );
    };
    match service.enqueue(request.clone()).await {
        Ok(job_id) => (
            StatusCode::ACCEPTED,
            Json(ReportAcknowledgement {
                status: "acknowledged",
                job_id,
                test_id: request.test_id,
                build_version: request.build_version,
                device_type: request.device_type,
                message: "Report generation queued",
            }),
        )
            .into_response(),
        Err(ReportError::Validation(message)) => {
            report_error(StatusCode::BAD_REQUEST, "validation", &message)
        }
        Err(ReportError::QueueFull | ReportError::WorkerUnavailable) => report_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "queue_unavailable",
            "Report queue is unavailable",
        ),
        Err(error) => {
            tracing::error!(%error, "failed to enqueue report");
            report_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Failed to queue report",
            )
        }
    }
}

#[utoipa::path(get, path = "/report/{job_id}", tag = "Reports", summary = "Get report job status", description = "Returns the current in-process report job snapshot; completed artifacts are referenced by the execution-report APIs.", params(("job_id" = Uuid, Path)),
    responses((status = 200, description = "Report job", body = ReportJobSnapshot), (status = 404, description = "Job not found", body = ErrorResponse), (status = 503, description = "Reports unavailable", body = ErrorResponse)))]
pub async fn report_status(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<Uuid>,
) -> axum::response::Response {
    let Some(service) = &state.reports else {
        return report_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            "Reports are unavailable",
        );
    };
    match service.status(job_id).await {
        Some(status) => Json(status).into_response(),
        None => report_error(StatusCode::NOT_FOUND, "not_found", "Job not found"),
    }
}

#[utoipa::path(post, path = "/upload-confluence", tag = "Reports", summary = "Upload generated report to Confluence", description = "Validates a root-contained generated report, creates or updates its Confluence page, and uploads referenced image attachments.", request_body = ConfluenceUploadRequest,
    responses((status = 202, description = "Report uploaded", body = ConfluenceUploadResponse), (status = 400, description = "Invalid request or report missing", body = ErrorResponse), (status = 500, description = "Confluence configuration or artifact access failed", body = ErrorResponse), (status = 502, description = "Confluence request failed", body = ErrorResponse), (status = 503, description = "Reports unavailable", body = ErrorResponse)))]
pub async fn upload_confluence(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfluenceUploadRequest>,
) -> axum::response::Response {
    let Some(service) = &state.reports else {
        return report_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            "Reports are unavailable",
        );
    };
    match service.upload_confluence(request).await {
        Ok(response) => (StatusCode::ACCEPTED, Json(response)).into_response(),
        Err(ConfluenceError::Validation(message)) => {
            report_error(StatusCode::BAD_REQUEST, "validation", &message)
        }
        Err(ConfluenceError::Configuration(message)) => {
            report_error(StatusCode::INTERNAL_SERVER_ERROR, "configuration", &message)
        }
        Err(ConfluenceError::Upstream(message)) => {
            report_error(StatusCode::BAD_GATEWAY, "confluence_upstream", &message)
        }
        Err(ConfluenceError::Io(error)) => {
            tracing::error!(%error, "Confluence artifact access failed");
            report_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "artifact_io",
                "Failed to access report artifacts",
            )
        }
    }
}

fn report_error(status: StatusCode, code: &str, message: &str) -> axum::response::Response {
    (
        status,
        Json(ErrorResponse {
            code: code.to_owned(),
            message: message.to_owned(),
            request_id: None,
        }),
    )
        .into_response()
}
