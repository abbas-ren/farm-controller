//! Integration tests for device HTTP routes and repository fakes.
//!
//! Exercises Axum handlers against in-memory repository doubles under `support/`.
//! Guards legacy Node `deviceRoutes` / analytics / execution wire contracts during the Rust port.

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

/// PURPOSE: Build shared fixture/helper `edge_controller_test_lock` for sibling tests in this module.
/// USE: test setup only; not used on production request paths.
fn edge_controller_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// PURPOSE: Build shared fixture/helper `build_performance_fixture` for sibling tests in this module.
/// USE: test setup only; not used on production request paths.
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
