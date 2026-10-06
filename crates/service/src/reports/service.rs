//! Bounded report queue, worker lifecycle, and job status orchestration.

use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use reqwest::Client;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    config::ReportsConfig,
    events::{EventHub, ServerEvent},
    observability::Metrics,
};

use super::{
    ReportDataSource, ReportError, ReportJobSnapshot, ReportRequest, ReportStatus, append_log,
    constants::service as messages, performance_graphs, report_root, write_combined_report,
    write_performance_comparisons, write_sanity_analysis, write_suite_artifacts,
};

type ReportQueueItem = (Uuid, ReportRequest);
type ReportReceiver = mpsc::Receiver<ReportQueueItem>;

#[derive(Clone)]
pub struct ReportService {
    pub(super) config: ReportsConfig,
    pub(super) jobs: Arc<RwLock<HashMap<Uuid, ReportJobSnapshot>>>,
    sender: mpsc::Sender<ReportQueueItem>,
    receiver: Arc<Mutex<Option<ReportReceiver>>>,
    data_source: Arc<dyn ReportDataSource>,
    metrics: Metrics,
    pub(super) client: Client,
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
        super::validate_request(&request)?;
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
        tracing::info!(%job_id, worker = "reports", "report job started");
        self.set_status(job_id, ReportStatus::Processing, None)
            .await;
        let result = self.generate(job_id, &request).await;
        match result {
            Ok(()) => {
                self.set_status(job_id, ReportStatus::Completed, None).await;
                self.metrics.record_worker_run("reports", "success");
                tracing::info!(%job_id, worker = "reports", "report job completed");
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
            &format!("{}: {job_id}", messages::GENERATION_STARTED),
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
                    &format!("{}: {error}", messages::PERFORMANCE_COMPARISON_SKIPPED),
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
                    &format!("{}: {error}", messages::PERFORMANCE_GRAPHS_SKIPPED),
                )
                .await?;
            } else {
                return Err(error.into());
            }
        }
        self.set_status(job_id, ReportStatus::BuildingReport, None)
            .await;
        write_combined_report(&root, request, &cases, &self.config.report_base_url).await?;
        append_log(&root, "INFO", messages::GENERATION_COMPLETED).await?;
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
}
