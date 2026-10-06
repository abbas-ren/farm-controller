use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicUsize, Ordering},
};

use axum::{body::Body, http::Request};
use clap::Parser;
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::repository::{
    AlertRepository, AnalyticsRepository, BuildRepository, ControllerRepository,
    DeviceArtifactRepository, DeviceInventoryRepository, DeviceRegistrationRepository,
    ExecutionLifecycleRepository, ExecutionQueryRepository, ExecutionReportRepository,
    FaultyReportRepository, HeartbeatRepository, PowerRelayRepository,
};
use super::*;
use crate::{
    api,
    auth::{
        AuthenticatedUser, IdentityError, IdentityProvider, ManagedUser, RegistrationRequest,
        SigninResult, ValidationResult,
    },
    cli::Cli,
    config::AppConfig,
    observability::Metrics,
    qmetry_catalog::{QmetryCatalog, QmetryError},
    reports::{ReportDataSource, ReportError, ReportService, TestCaseRecord},
    test_catalog::{TestCase, TestCatalog, TestCatalogError, TestPlan, TestSuite},
};

struct RegistrationRepository {
    calls: AtomicUsize,
    callbacks: Mutex<Vec<String>>,
}

struct AcceptingIdentityProvider;

struct FakeTestCatalog;
struct FakeQmetryCatalog;
struct FakeReportDataSource;

fn edge_controller_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn build_performance_fixture(build_id: &str) -> serde_json::Value {
    serde_json::json!({
        "buildId": build_id,
        "buildVersion": "v1",
        "flagged": false,
        "deviceType": "Racer",
        "status": "completed",
        "lastTestExecutionId": "test-1",
        "passedTestCaseCount": 1,
        "failedTestCaseCount": 1,
        "passedTestCasePercentage": 50.0,
        "averageExecutionTime": 60000,
        "uniqueDeviceCount": 1,
        "executionCount": 1,
    })
}

mod analytics;
mod build_upload;
mod controllers;
mod device_routes;
mod events;
mod execution_logs;
mod exports_callbacks;
mod notifications_reports;
mod support;
mod test_catalog;
mod test_execution;
