//! Report API facade; implementation is organized by responsibility.

mod artifact_writers;
mod combined_report;
mod confluence;
mod constants;
mod data_source;
mod errors;
mod http;
mod models;
mod performance;
mod performance_graphs;
mod service;
mod validation;

#[cfg(test)]
pub mod tests;

pub use data_source::{PostgresReportDataSource, ReportDataSource};
pub use errors::{ConfluenceError, ReportError};
pub use http::{
    __path_enqueue_report, __path_report_status, __path_upload_confluence, enqueue_report,
    report_status, router, upload_confluence,
};
pub use models::{
    ConfluenceUploadRequest, ConfluenceUploadResponse, ReportAcknowledgement, ReportJobSnapshot,
    ReportRequest, ReportStatus, TestCaseRecord,
};
pub use service::ReportService;

#[cfg(test)]
use crate::{config::ReportsConfig, events::EventHub, observability::Metrics};
#[cfg(test)]
use async_trait::async_trait;
#[cfg(test)]
pub(crate) use confluence::{ConfluenceSettings, confluence_body};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use tokio::sync::Mutex;
#[cfg(test)]
use tokio_util::sync::CancellationToken;
#[cfg(test)]
use uuid::Uuid;

pub(super) use artifact_writers::{
    append_log, escape_html, file_name, write_sanity_analysis, write_suite_artifacts,
};
pub(super) use combined_report::write_combined_report;
pub(super) use performance::{performance_metrics, write_performance_comparisons};
pub(super) use validation::{report_root, validate_request};
