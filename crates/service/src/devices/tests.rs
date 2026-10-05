use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicUsize, Ordering},
};

use axum::{body::Body, http::Request};
use clap::Parser;
use http_body_util::BodyExt;
use tower::ServiceExt;

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

#[test]
fn device_power_event_preserves_frontend_contract() {
    let event = device_power_event("device-1", "off");
    assert_eq!(event.event, "device_power_update");
    assert_eq!(event.payload["deviceId"], "device-1");
    assert_eq!(event.payload["power"], "off");
    assert!(event.payload["changedAt"].is_string());
    assert!(event.room.is_none());
}

#[test]
fn new_execution_event_preserves_frontend_contract() {
    let event = test_execution_handlers::new_execution_event("test-1", "user-1");
    assert_eq!(event.event, "test_execution");
    assert_eq!(event.payload["testId"], "test-1");
    assert_eq!(event.payload["type"], "new");
    assert_eq!(event.payload["data"]["status"], "not_executed");
    assert_eq!(
        event.payload["data"]["message"],
        "Test execution test-1 created"
    );
    assert!(event.payload["data"]["timestamp"].is_string());
    assert_eq!(event.room.as_deref(), Some("user:user-1"));
}

#[test]
fn cancellation_events_preserve_user_and_build_contracts() {
    let [user_event, build_event] = test_execution_handlers::cancellation_events(
        "test-1",
        "build-1",
        "PREPARE_ARTIFACTS",
        "user-1",
    );
    assert_eq!(user_event.event, "test_execution");
    assert_eq!(user_event.payload["type"], "cancel_requested");
    assert_eq!(user_event.payload["data"]["cancelRequested"], true);
    assert_eq!(user_event.payload["data"]["phase"], "PREPARE_ARTIFACTS");
    assert_eq!(user_event.room.as_deref(), Some("user:user-1"));
    assert_eq!(build_event.event, "test_execution_update");
    assert_eq!(build_event.payload["testId"], "test-1");
    assert_eq!(build_event.payload["buildId"], "build-1");
    assert_eq!(build_event.payload["cancelRequested"], true);
    assert_eq!(build_event.room.as_deref(), Some("build:build-1"));
}

#[test]
fn report_upload_events_preserve_frontend_status_and_room_contracts() {
    let report_id = Uuid::new_v4();
    let events = test_execution_handlers::report_update_events(
        report_id,
        "test-1",
        "user-1",
        "failed",
        Some("Confluence upload failed: rejected"),
        Some("rejected"),
    );
    assert_eq!(events[0].event, "execution_report_update");
    assert_eq!(events[0].payload["reportId"], report_id.to_string());
    assert_eq!(events[0].payload["testExecutionId"], "test-1");
    assert_eq!(events[0].payload["status"], "failed");
    assert_eq!(events[0].payload["createdBy"], "user-1");
    assert_eq!(
        events[0].payload["error"],
        "Confluence upload failed: rejected"
    );
    assert_eq!(events[0].payload["uploadError"], "rejected");
    assert_eq!(events[0].room.as_deref(), Some("user:user-1"));
    assert_eq!(events[1].payload, events[0].payload);
    assert_eq!(events[1].room.as_deref(), Some("test:test-1"));
}

#[test]
fn relay_confirmation_events_follow_committed_context() {
    let response = RelayConfirmationResponse {
        confirmed: true,
        message: "confirmed".to_owned(),
        controller_id: Some("controller-1".to_owned()),
        relay_id: Some(Uuid::nil()),
        device_id: Some("device-1".to_owned()),
        configuration_complete: true,
    };
    let events = relay_handlers::confirmation_events(&response);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].event, "device_power_update");
    assert_eq!(events[0].payload["deviceId"], "device-1");
    assert_eq!(events[1].event, "relay_configuration_status");
    assert_eq!(events[1].payload["status"], "completed");
    assert_eq!(events[1].payload["completed"], 1);
    assert_eq!(events[2].event, "device_controller_changed");
    assert_eq!(events[2].payload["controllerId"], "controller-1");
    assert!(events.iter().all(|event| event.room.is_none()));
}

#[async_trait]
impl ReportDataSource for FakeReportDataSource {
    async fn test_cases(&self, _test_id: &str) -> Result<Vec<TestCaseRecord>, ReportError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl QmetryCatalog for FakeQmetryCatalog {
    async fn cases_by_plan(&self, _plan_id: u64) -> Result<serde_json::Value, QmetryError> {
        Ok(
            serde_json::json!({"8": [{"id": "case-1", "name": "Boot", "script_file": "run.sh", "labels": ["smoke"]}]}),
        )
    }

    async fn cases_by_folder(&self, _folder_id: u64) -> Result<serde_json::Value, QmetryError> {
        Ok(serde_json::json!([{"id": "case-1", "name": "Boot", "script_file": "run.sh"}]))
    }
}

#[async_trait]
impl TestCatalog for FakeTestCatalog {
    async fn plans(&self, filter: Option<&str>) -> Result<Vec<TestPlan>, TestCatalogError> {
        Ok(vec![TestPlan {
            id: 7,
            name: filter.unwrap_or("Plan").to_owned(),
            description: Some("Device smoke plan".to_owned()),
            test_suits: Vec::new(),
        }])
    }

    async fn suites(&self, plan_id: u64) -> Result<Vec<TestSuite>, TestCatalogError> {
        Ok(vec![TestSuite {
            id: 8,
            name: "Core".to_owned(),
            plan_id,
            order: 1,
            description: None,
        }])
    }

    async fn cases(
        &self,
        plan_id: u64,
        suite_id: u64,
        _filter: Option<&str>,
    ) -> Result<Vec<TestCase>, TestCatalogError> {
        Ok(vec![TestCase {
            id: 9,
            title: "Boot".to_owned(),
            plan_id,
            suite_id,
            order: 1,
            priority_id: 2,
            script_file: "run.sh".to_owned(),
            pre_condition: Some("Ready".to_owned()),
            labels: vec!["smoke".to_owned()],
        }])
    }

    async fn update_result(
        &self,
        _update: &crate::test_catalog::TestResultUpdate,
    ) -> Result<(), TestCatalogError> {
        Ok(())
    }
}

#[async_trait]
impl IdentityProvider for AcceptingIdentityProvider {
    async fn signin(
        &self,
        _username: &str,
        _password: &str,
    ) -> Result<SigninResult, IdentityError> {
        Ok(SigninResult {
            access_token: "access".to_owned(),
            refresh_token: "refresh".to_owned(),
            user: AuthenticatedUser {
                id: "user-1".to_owned(),
                username: "farm.user".to_owned(),
                email: "farm.user@example.com".to_owned(),
                first_name: None,
                last_name: None,
                realm_roles: vec!["user".to_owned()],
            },
        })
    }

    async fn validate(
        &self,
        _access_token: &str,
        _refresh_token: Option<&str>,
        _required_role: Option<&str>,
    ) -> Result<ValidationResult, IdentityError> {
        Ok(ValidationResult {
            refreshed_tokens: None,
        })
    }

    async fn request_registration(
        &self,
        request: &RegistrationRequest,
    ) -> Result<serde_json::Value, IdentityError> {
        Ok(serde_json::json!({
            "id": "pending-user-1",
            "username": request.username,
            "firstName": request.first_name,
            "lastName": request.last_name,
            "email": request.email,
            "emailVerified": false,
            "createdTimestamp": 1790899200000_i64,
            "enabled": true,
            "totp": false,
            "disableableCredentialTypes": [],
            "requiredActions": ["UPDATE_PASSWORD"],
            "notBefore": 0,
            "access": {},
            "realmRoles": []
        }))
    }

    async fn user_by_id(&self, user_id: &str) -> Result<ManagedUser, IdentityError> {
        Ok(ManagedUser {
            id: user_id.to_owned(),
            username: "farm.user".to_owned(),
            email: Some("farm.user@example.com".to_owned()),
            first_name: Some("Farm".to_owned()),
            last_name: Some("User".to_owned()),
            realm_roles: vec!["user".to_owned()],
        })
    }
}

#[async_trait]
impl DeviceRepository for RegistrationRepository {
    async fn create_test_execution(
        &self,
        execution: &ExecutionCreation,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(serde_json::json!({
            "id": 1,
            "testId": "created-test-1",
            "deviceFamily": execution.device_family,
            "deviceType": execution.device_type,
            "deviceId": null,
            "buildId": execution.build_id,
            "testPlanId": execution.plan_id,
            "testSuits": execution.test_suites,
            "testCases": execution.cases,
            "testPlanName": execution.plan_name,
            "isAllSelected": execution.is_all_selected,
            "status": "not_executed",
            "executionPhase": "PREPARE_ARTIFACTS",
            "createdBy": execution.user_id,
            "selectionInput": execution.selection,
            "Device": null,
            "logs": []
        }))
    }

    async fn test_execution_build_version(
        &self,
        test_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        Ok((test_id == "42").then(|| "v1.2.3".to_owned()))
    }

    async fn create_log(
        &self,
        request: &LogCreateRequest,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(serde_json::json!({
            "id": 1,
            "type": request.log_type,
            "referenceId": request.reference_id,
            "data": request.data,
            "level": request.level,
            "timestamp": request.timestamp.unwrap_or_else(chrono::Utc::now)
        }))
    }

    async fn list_logs(
        &self,
        query: &LogListQuery,
        search: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
        Ok(serde_json::json!({
            "logs": [{
                "id": 1,
                "type": query.log_type.as_deref().unwrap_or("general"),
                "referenceId": query.reference_id.as_deref().unwrap_or("system"),
                "data": {"message": search.unwrap_or("ready")},
                "level": query.level.as_deref().unwrap_or("info")
            }],
            "total": 1,
            "page": page,
            "pageSize": page_size
        }))
    }

    async fn executions_by_build(
        &self,
        build_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(
            serde_json::json!({"limit": limit, "offset": offset, "total": 1, "executions": [{"testId": "test-1", "buildId": build_id, "testCases": []}]}),
        )
    }

    async fn test_results(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((test_id == "test-1")
            .then(|| serde_json::json!({"status": "completed", "testCases": [{"result": "PASS"}]})))
    }

    async fn request_test_cancellation(
        &self,
        test_id: &str,
        _user_id: &str,
        _nfs_host_path: &str,
    ) -> Result<Option<TestCancellationResult>, DeviceRepositoryError> {
        Ok((test_id == "test-1").then(|| TestCancellationResult {
            build_id: "build-1".to_owned(),
            created_by: "user-1".to_owned(),
            phase: "PREPARE_ARTIFACTS".to_owned(),
            changed: true,
            terminalized: true,
            device_ip: None,
            cleanup_device_type: None,
            test_cycle_id: None,
        }))
    }

    async fn report_generation_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError> {
        Ok((test_id == "test-1").then(|| ("x5h".to_owned(), "v1.2.3".to_owned())))
    }

    async fn create_execution_report_record(
        &self,
        report_id: Uuid,
        test_id: &str,
        device_type: &str,
        build_version: &str,
        _user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(
            serde_json::json!({"id": report_id, "testExecutionId": test_id, "deviceType": device_type, "buildVersion": build_version, "status": "generating"}),
        )
    }

    async fn update_execution_report_status(
        &self,
        _report_id: Uuid,
        _status: &str,
        _error: Option<&str>,
    ) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }

    async fn execution_report(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((test_id == "test-1").then(|| serde_json::json!({"id": "report-1", "testExecutionId": "test-1", "status": "completed"})))
    }

    async fn execution_report_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError> {
        Ok((test_id == "test-1").then(|| ("x5h".to_owned(), "v1.2.3".to_owned())))
    }

    async fn test_case_log_path(
        &self,
        case_id: i64,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        Ok((case_id == 1).then(|| "x5h/v1.2.3/test-1/case.log".to_owned()))
    }

    async fn execution_cases(
        &self,
        _user_id: &str,
        test_id: &str,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
        Ok((test_id == "test-1").then(|| {
            vec![serde_json::json!({"id": 1, "testCaseId": 11, "title": "Boot", "result": "PASS"})]
        }))
    }

    async fn test_case(
        &self,
        case_id: i64,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((case_id == 1).then(|| serde_json::json!({"id": 1, "testCaseId": 11, "title": "Boot"})))
    }

    async fn list_test_executions(
        &self,
        _user_id: &str,
        query: &ExecutionListQuery,
    ) -> Result<ExecutionList, DeviceRepositoryError> {
        Ok(ExecutionList {
            data: vec![
                serde_json::json!({"testId": "test-1", "durationSeconds": 60, "total": 1, "passed": 1, "failed": 0, "deviceName": "x5h", "testCases": {}, "logs": []}),
            ],
            total: 1,
            current_page: query.page.unwrap_or(1).max(1) as u64,
            total_pages: 1,
        })
    }

    async fn test_execution_by_device(
        &self,
        _user_id: &str,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((device_id == "device-1").then(|| serde_json::json!({"testId": "test-1", "deviceId": "device-1", "status": "in_progress"})))
    }

    async fn in_progress_test_executions(
        &self,
        _user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![
            serde_json::json!({"testId": "test-1", "deviceId": "device-1", "status": "in_progress"}),
        ])
    }

    async fn single_test_execution(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((test_id == "test-1")
            .then(|| serde_json::json!({"testId": "test-1", "testCases": [{"testCaseId": 11}]})))
    }

    async fn execution_case_ids(
        &self,
        test_id: &str,
    ) -> Result<Option<Vec<String>>, DeviceRepositoryError> {
        Ok((test_id == "test-1").then(|| vec!["11".to_owned(), "12".to_owned()]))
    }

    async fn test_execution(
        &self,
        _user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((execution_id == "test-1" || execution_id == "latest").then(|| serde_json::json!({"testId": "test-1", "status": "in_progress", "Device": {"deviceType": "x5h"}})))
    }

    async fn test_execution_summary(
        &self,
        _user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((execution_id == "test-1").then(|| {
            serde_json::json!({
                "testId": "test-1", "durationSeconds": 60, "total": 1,
                "passed": 1, "failed": 0, "buildId": "build-1",
                "buildVersion": "v1.2.3", "deviceName": "x5h", "deviceType": "x5h",
                "status": "completed", "testCases": [{"id": 1, "testCaseId": 11, "result": "PASS"}],
                "testPlanName": "Smoke", "testCycleId": "42",
                "analytics": {
                    "allTime": {"total": 1, "passed": 1, "failed": 0, "inProgress": 0},
                    "currentWeek": {"total": 1, "passed": 1, "failed": 0, "inProgress": 0},
                    "currentMonth": {"total": 1, "passed": 1, "failed": 0, "inProgress": 0}
                },
                "rtosLogPath": null
            })
        }))
    }

    async fn test_export_rows(
        &self,
        _user_id: &str,
    ) -> Result<Vec<TestExportRow>, DeviceRepositoryError> {
        Ok(vec![TestExportRow {
            test_plan_name: "Smoke".to_owned(),
            status: "completed".to_owned(),
            device_type: Some("x5h".to_owned()),
            started_at: Some(
                chrono::DateTime::parse_from_rfc3339("2026-10-02T00:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
            ended_at: Some(
                chrono::DateTime::parse_from_rfc3339("2026-10-02T00:01:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
            test_cycle_id: None,
            total: 1,
            passed: 1,
            failed: 0,
        }])
    }

    async fn flag_build(
        &self,
        build_id: &str,
        request: &BuildFlagRequest,
    ) -> Result<Option<repository_types::BuildFlagResult>, DeviceRepositoryError> {
        Ok((build_id == "build-1").then(|| repository_types::BuildFlagResult {
            version: "v1.2.3".to_owned(),
            device_type: "x5h".to_owned(),
            alert: serde_json::json!({
                "title": "Build Flagged", "type": if request.is_faulty { "warning" } else { "success" }
            }),
        }))
    }

    async fn delete_build(
        &self,
        build_id: &str,
    ) -> Result<Option<BuildDeleteTarget>, DeviceRepositoryError> {
        Ok((build_id == "build-1").then(|| BuildDeleteTarget {
            folder_name: "Gen5_x5h".to_owned(),
            version: "v1.2.3".to_owned(),
            device_family: "Gen5".to_owned(),
            device_type: "x5h".to_owned(),
            alert: serde_json::json!({"title": "Build Deleted", "type": "error"}),
        }))
    }

    async fn finalize_build_upload(
        &self,
        request: &BuildUploadFinalization,
    ) -> Result<BuildUploadResult, DeviceRepositoryError> {
        if request.filename == "fail.zip" {
            return Err(DeviceRepositoryError::Internal(
                "upload processing failed".to_owned(),
            ));
        }
        Ok(BuildUploadResult {
            release: serde_json::json!({
                "id": "build-upload",
                "version": "v1.2.3",
                "deviceType": "x5h",
                "deviceFamily": "Gen5",
                "buildType": if request.is_custom { "custom" } else { "official" }
            }),
            alert: serde_json::json!({
                "title": "Build Upload Success",
                "type": "success"
            }),
        })
    }

    async fn init_build_upload(
        &self,
        _file_count: i32,
        _user_id: &str,
    ) -> Result<Uuid, DeviceRepositoryError> {
        Ok(Uuid::nil())
    }

    async fn mark_build_upload_started(
        &self,
        _upload_id: &str,
        _user_id: &str,
    ) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }

    async fn mark_build_upload_failed(
        &self,
        _upload_id: &str,
    ) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }

    async fn build_filters(&self) -> Result<BuildFilters, DeviceRepositoryError> {
        Ok(BuildFilters {
            device_types: vec!["x5h".to_owned()],
            device_families: vec!["Gen5".to_owned()],
        })
    }

    async fn build_by_id(
        &self,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((build_id == "build-1").then(|| serde_json::json!({"id": "build-1", "version": "1.2.3", "deviceFamily": "Gen5", "deviceType": "x5h"})))
    }

    async fn list_builds(
        &self,
        query: &BuildListQuery,
    ) -> Result<BuildList, DeviceRepositoryError> {
        Ok(BuildList {
            builds: vec![
                serde_json::json!({"id": "build-1", "version": "1.2.3", "deviceFamily": "Gen5", "deviceType": "x5h"}),
            ],
            total_count: 1,
            current_page: query.page.unwrap_or(1).max(1) as u64,
            total_pages: 1,
            requested_count: 1,
        })
    }

    async fn test_completion_target(
        &self,
        test_id: &str,
        _device_id: &str,
    ) -> Result<Option<repository_types::TestCompletionTarget>, DeviceRepositoryError> {
        Ok(
            (test_id == "test-1").then(|| repository_types::TestCompletionTarget {
                status: "in_progress".to_owned(),
                device_family: Some("Gen3".to_owned()),
                mac_address: Some("00:11:22:33:44:55".to_owned()),
                device_type: "x3h".to_owned(),
                build_version: Some("v1.2.3".to_owned()),
                build_id: Some("build-1".to_owned()),
                created_by: Some("user-1".to_owned()),
                gen5_controller_ip: None,
                uart_port: None,
                gen4_controller_ip: None,
                relay_serial: None,
                relay_channel: None,
            }),
        )
    }

    async fn store_rtos_log_path(
        &self,
        _test_id: &str,
        _path: &str,
    ) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }

    async fn mark_device_flashing(
        &self,
        device_id: &str,
    ) -> Result<Option<repository_types::DeviceFlashingResult>, DeviceRepositoryError> {
        Ok(
            (device_id == "device-1").then(|| repository_types::DeviceFlashingResult {
                device: serde_json::json!({
                    "deviceId": device_id, "state": "busy",
                    "upgrading": true, "flashing": false
                }),
                rtos_target: None,
            }),
        )
    }

    async fn export_devices(
        &self,
        _query: &DeviceCsvQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "deviceId": "device-1",
            "deviceName": "Bench 1",
            "deviceType": "x5h",
            "macAddress": "00:11:22:33:44:55",
            "ipAddress": "192.0.2.10",
            "status": "approved",
            "state": "free",
            "createdAt": "2026-10-02T12:00:00Z",
            "updatedAt": "2026-10-03T12:00:00Z",
            "lastConnectedOn": "2026-10-03T12:00:00Z",
            "interfaces": [{"type": "ethernet", "interfaceId": "eth0"}]
        })])
    }

    async fn register_device(
        &self,
        registration: &DeviceRegistration,
    ) -> Result<repository_types::DeviceRegistrationResult, DeviceRepositoryError> {
        let device_id = validation::validate_device_registration(registration)?;
        Ok(repository_types::DeviceRegistrationResult {
            device: serde_json::json!({
                "deviceId": device_id,
                "macAddress": registration.mac_address,
                "ipAddress": registration.ip_address,
                "deviceName": registration.device_name,
                "state": registration.state,
                "status": registration.status,
                "heartbeatTimer": registration.timeout,
                "interfaces": validation::device_interfaces(registration)
                    .into_iter()
                    .map(|interface| serde_json::json!({
                        "deviceId": device_id,
                        "type": interface.interface_type,
                        "interfaceId": interface.interface_id
                    }))
                    .collect::<Vec<_>>()
            }),
            notify_addition: true,
            notify_approval: true,
        })
    }

    async fn register_controller(
        &self,
        registration: &ControllerRegistration,
    ) -> Result<RegistrationResult, DeviceRepositoryError> {
        let created = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let now = chrono::Utc::now();
        Ok(RegistrationResult {
            created,
            controller: DeviceController {
                device_controller_id: validation::normalized_controller_id(
                    &registration.mac_address,
                )?,
                mac_address: registration.mac_address.clone(),
                ip_address: registration.ip_address.clone(),
                name: None,
                device_family: registration.device_family.clone(),
                status: "approved".to_owned(),
                state: "active".to_owned(),
                created_at: now,
                updated_at: now,
                relays: Vec::new(),
            },
        })
    }

    async fn save_gen5_mapping(
        &self,
        caller_ip: &str,
        mac: &str,
        status: CallbackStatus,
        tty_entry: Option<&TtyEntry>,
    ) -> Result<(), DeviceRepositoryError> {
        self.callbacks.lock().unwrap().push(format!(
            "mapping:{caller_ip}:{mac}:{status:?}:{}",
            tty_entry.map_or("none", |entry| entry.uart.as_str())
        ));
        Ok(())
    }

    async fn confirm_flash(
        &self,
        device_type: &str,
        status: CallbackStatus,
    ) -> Result<FlashConfirmationResult, DeviceRepositoryError> {
        self.callbacks
            .lock()
            .unwrap()
            .push(format!("flash:{device_type}:{status:?}"));
        Ok(if status == CallbackStatus::Failure {
            FlashConfirmationResult {
                cancelled_execution: Some(repository_types::FlashCancelledExecution {
                    test_id: "test-1".to_owned(),
                    build_id: "build-1".to_owned(),
                    created_by: "user-1".to_owned(),
                }),
                freed_device_id: Some("device-1".to_owned()),
            }
        } else {
            FlashConfirmationResult::default()
        })
    }

    async fn device_families(&self) -> Result<Vec<String>, DeviceRepositoryError> {
        Ok(vec!["Gen4".to_owned(), "Gen5".to_owned()])
    }

    async fn device_types(
        &self,
        device_family: Option<&str>,
    ) -> Result<Vec<String>, DeviceRepositoryError> {
        Ok(match device_family {
            Some("Gen5") => vec!["x5h".to_owned()],
            _ => vec!["v4h".to_owned(), "x5h".to_owned()],
        })
    }

    async fn device_by_id(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((device_id == "device-1").then(|| {
            serde_json::json!({
                "deviceId": "device-1",
                "deviceFamily": "Gen5",
                "interfaces": [],
                "testExecutions": []
            })
        }))
    }

    async fn latest_heartbeat(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((device_id == "device-1").then(|| {
            serde_json::json!({
                "timestamp": "2026-10-02T00:00:00Z",
                "data": {"cpuUsagePercent": 12.5},
                "timeout": 30
            })
        }))
    }

    async fn topology(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "deviceId": "device-1",
            "deviceName": "Bench 1",
            "deviceType": "x5h",
            "deviceFamily": "Gen5",
            "macAddress": "001122334455",
            "ipAddress": "192.0.2.20",
            "state": "free",
            "status": "approved",
            "controllerId": "aabbccddeeff",
            "heartbeatTimer": 5
        })])
    }

    async fn list_devices(
        &self,
        query: &DeviceListQuery,
        is_admin: bool,
    ) -> Result<DeviceList, DeviceRepositoryError> {
        Ok(DeviceList {
            data: vec![
                serde_json::json!({"deviceId": "device-1", "status": if is_admin { "requested" } else { "approved" }}),
            ],
            total_pages: 1,
            current_page: query.page.unwrap_or(1),
            total_devices: 1,
            requested_count: u64::from(is_admin),
            device_timers: BTreeMap::from([("device-1".to_owned(), 30)]),
            device_timeouts: BTreeMap::from([("device-1".to_owned(), 5)]),
            state_count: BTreeMap::new(),
        })
    }

    async fn device_action_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceActionTarget>, DeviceRepositoryError> {
        Ok(
            matches!(device_id, "device-1" | "heartbeat-device").then(|| DeviceActionTarget {
                device_id: device_id.to_owned(),
                ip_address: if device_id == "heartbeat-device" {
                    "127.0.0.1".to_owned()
                } else {
                    "192.0.2.20".to_owned()
                },
                status: "requested".to_owned(),
            }),
        )
    }

    async fn apply_device_action(
        &self,
        device_id: &str,
        action: DeviceAction,
        _user_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let status = match action {
            DeviceAction::Approved => "approved",
            DeviceAction::Declined => "declined",
        };
        Ok((device_id == "device-1")
            .then(|| serde_json::json!({"deviceId": device_id, "status": status})))
    }

    async fn list_controllers(
        &self,
        query: &ControllerListQuery,
    ) -> Result<ControllerList, DeviceRepositoryError> {
        Ok(ControllerList {
            data: Vec::new(),
            total_pages: 0,
            current_page: query.page.unwrap_or(1),
            total_count: 0,
            summary: serde_json::json!({"controllers": {"total": 0, "active": 0, "notReachable": 0}, "relays": {"total": 0, "connected": 0, "disconnected": 0}}),
        })
    }

    async fn list_user_devices(
        &self,
        query: &UserDeviceListQuery,
    ) -> Result<DeviceList, DeviceRepositoryError> {
        Ok(DeviceList {
            data: Vec::new(),
            total_pages: 0,
            current_page: query.page.unwrap_or(1),
            total_devices: 0,
            requested_count: 0,
            device_timers: BTreeMap::new(),
            device_timeouts: BTreeMap::new(),
            state_count: BTreeMap::new(),
        })
    }

    async fn active_devices(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(Vec::new())
    }

    async fn update_heartbeat_timeout(
        &self,
        device_id: &str,
        value: i64,
    ) -> Result<bool, DeviceRepositoryError> {
        self.callbacks
            .lock()
            .unwrap()
            .push(format!("heartbeat:{device_id}:{value}"));
        Ok(true)
    }

    async fn edit_controller(
        &self,
        _controller_id: &str,
        _request: &ControllerEditRequest,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(None)
    }

    async fn controller_delete_target(
        &self,
        _controller_id: &str,
    ) -> Result<Option<ControllerDeleteTarget>, DeviceRepositoryError> {
        Ok(None)
    }

    async fn delete_controller(&self, _controller_id: &str) -> Result<bool, DeviceRepositoryError> {
        Ok(false)
    }

    async fn device_delete_target(
        &self,
        _device_id: &str,
    ) -> Result<Option<DeviceDeleteTarget>, DeviceRepositoryError> {
        Ok(None)
    }

    async fn delete_device(&self, _device_id: &str) -> Result<bool, DeviceRepositoryError> {
        Ok(false)
    }

    async fn builds_for_device_type(
        &self,
        _device_type: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(Vec::new())
    }

    async fn configure_artifacts(
        &self,
        entries: &[DeviceTypeFolder],
    ) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError> {
        Ok(entries.to_vec())
    }

    async fn artifact_folders(&self) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError> {
        Ok(Vec::new())
    }

    async fn artifact_folder_for_device(
        &self,
        _device_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        Ok(None)
    }

    async fn device_reboot_target(
        &self,
        _device_id: &str,
    ) -> Result<Option<DeviceRebootTarget>, DeviceRepositoryError> {
        Ok(None)
    }

    async fn device_power_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError> {
        Ok((device_id == "power-device").then(|| DevicePowerTarget {
            device_family: Some("Gen4".to_owned()),
            current_power: Some("on".to_owned()),
            controller_ip: Some("127.0.0.1".to_owned()),
            power_port: None,
            relay_serial: Some("relay-1".to_owned()),
            channel_id: Some(Uuid::nil()),
            channel_number: Some(2),
        }))
    }

    async fn apply_device_power(
        &self,
        device_id: &str,
        channel_id: Option<Uuid>,
        state: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        if device_id != "power-device" {
            return Ok(None);
        }
        self.callbacks
            .lock()
            .unwrap()
            .push(format!("power:{device_id}:{state}"));
        Ok(Some(serde_json::json!({
            "id": channel_id,
            "deviceId": device_id,
            "channelNumber": 2,
            "state": state,
        })))
    }

    async fn relays_for_controller(
        &self,
        _controller_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(Vec::new())
    }

    async fn channels_for_relay(
        &self,
        _relay_id: Uuid,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
        Ok(Some(Vec::new()))
    }

    async fn available_relay_devices(
        &self,
        query: &AvailableRelayDevicesQuery,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        self.callbacks.lock().unwrap().push(format!(
            "available:{}:{}:{}",
            query.page.unwrap_or(1),
            query.limit.unwrap_or(10),
            query
                .relay_id
                .map(|relay_id| relay_id.to_string())
                .unwrap_or_default()
        ));
        Ok(serde_json::json!({"devices": [{
            "deviceId": "device-1",
            "macAddress": "00:11:22:33:44:55",
            "deviceName": "Bench device",
            "ipAddress": "192.0.2.10",
            "deviceType": "Racer",
            "deviceFamily": "Gen4"
        }], "pagination": {
            "page": query.page.unwrap_or(1),
            "limit": query.limit.unwrap_or(10),
            "totalCount": 1,
            "totalPages": 1,
        }}))
    }

    async fn relay_state_target(
        &self,
        _device_id: &str,
    ) -> Result<Option<RelayStateTarget>, DeviceRepositoryError> {
        Ok(None)
    }

    async fn update_relay_state(
        &self,
        _channel_id: Uuid,
        _state: &str,
    ) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }

    async fn relay_identity_target(
        &self,
        _relay_id: Uuid,
    ) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError> {
        Ok(Some(RelayIdentityTarget {
            controller_id: "controller-1".to_owned(),
            controller_ip: "127.0.0.1".to_owned(),
            current_serial: "relay-1".to_owned(),
        }))
    }

    async fn update_relay_identity(
        &self,
        _relay_id: Uuid,
        _old_serial: &str,
        _serial_number: &str,
        _vendor_id: &str,
        _product_id: &str,
    ) -> Result<bool, DeviceRepositoryError> {
        Ok(true)
    }

    async fn configure_relay_channels(
        &self,
        configurations: &[RelayChannelConfiguration],
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        Ok(RelayConfigurationResult {
            channels: configurations
                .iter()
                .map(|configuration| {
                    serde_json::json!({
                        "id": configuration.channel_id,
                        "relayId": configuration.relay_id,
                        "deviceId": configuration.device_id,
                    })
                })
                .collect(),
            actions: Vec::new(),
        })
    }

    async fn confirm_relay_configuration(
        &self,
        mac: &str,
        succeeded: bool,
    ) -> Result<RelayConfirmationResponse, DeviceRepositoryError> {
        Ok(RelayConfirmationResponse {
            confirmed: succeeded,
            message: format!("Configuration callback for {mac}"),
            controller_id: Some("controller-1".to_owned()),
            relay_id: Some(Uuid::nil()),
            device_id: Some("device-1".to_owned()),
            configuration_complete: succeeded,
        })
    }

    async fn legacy_relays(
        &self,
        query: &LegacyRelayListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "id": Uuid::nil(),
            "deviceControllerId": query.controller_id,
            "serialNumber": "relay-1",
        })])
    }

    async fn legacy_relay_channels(
        &self,
        query: &LegacyRelayChannelListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "id": Uuid::nil(),
            "relayId": query.relay_id,
            "deviceId": query.device_id,
            "channelNumber": 0,
        })])
    }

    async fn legacy_relay_by_id(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(Some(
            serde_json::json!({"id": relay_id, "serialNumber": "relay-1"}),
        ))
    }

    async fn legacy_relay_channel_by_id(
        &self,
        channel_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(Some(
            serde_json::json!({"id": channel_id, "channelNumber": 0}),
        ))
    }

    async fn delete_legacy_relay(&self, _relay_id: Uuid) -> Result<bool, DeviceRepositoryError> {
        Ok(true)
    }

    async fn delete_legacy_relay_channel(
        &self,
        _channel_id: Uuid,
    ) -> Result<bool, DeviceRepositoryError> {
        Ok(true)
    }

    async fn configure_legacy_relay(
        &self,
        request: &LegacyRelayConfiguration,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        Ok(RelayConfigurationResult {
            channels: vec![serde_json::json!({
                "id": request.id,
                "relayId": request.relay_id,
                "deviceId": request.device_id,
            })],
            actions: Vec::new(),
        })
    }

    async fn fresh_legacy_relay(&self, _relay_id: Uuid) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }

    async fn remap_legacy_relay(
        &self,
        _relay_id: Uuid,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        Ok(RelayConfigurationResult {
            channels: Vec::new(),
            actions: Vec::new(),
        })
    }

    async fn legacy_relay_conflicts(
        &self,
        query: &LegacyRelayConflictQuery,
    ) -> Result<RelayConflictState, DeviceRepositoryError> {
        Ok(RelayConflictState {
            device_conflict: query.device_id.is_some(),
            relay_conflict: false,
            channel_conflict: query.channel_number == 1,
        })
    }

    async fn device_state_analytics(&self) -> Result<DeviceStateAnalytics, DeviceRepositoryError> {
        Ok(DeviceStateAnalytics {
            total: 2,
            state_count: BTreeMap::from([("busy".to_owned(), 1), ("free".to_owned(), 1)]),
        })
    }

    async fn detailed_device_state_analytics(
        &self,
    ) -> Result<DeviceStateDetailedAnalytics, DeviceRepositoryError> {
        Ok(DeviceStateDetailedAnalytics(BTreeMap::from([(
            "Gen5".to_owned(),
            vec![DeviceStateDetailedItem {
                device_id: "device-1".to_owned(),
                device_type: "Racer".to_owned(),
                state: "free".to_owned(),
            }],
        )])))
    }

    async fn recent_execution_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "testId": "test-1",
            "buildVersion": "v1",
            "status": "completed",
            "testPlanName": "Smoke",
            "createdAt": "2026-10-02T00:00:00Z",
            "deviceType": "Racer",
            "startedAt": "2026-10-02T00:00:00Z",
            "endedAt": "2026-10-02T00:01:00Z",
            "totalTestCases": count.min(2),
            "createdBy": user_id,
        })])
    }

    async fn execution_analytics_by_id(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((test_id == "test-1").then(|| {
            serde_json::json!({
                "testId": test_id,
                "buildVersion": "v1",
                "status": "completed",
                "testPlanName": "Smoke",
                "createdAt": "2026-10-02T00:00:00Z",
                "totalTestCases": 2,
            })
        }))
    }

    async fn daily_execution_analytics(
        &self,
        _user_id: &str,
        from: chrono::NaiveDate,
        _to: chrono::NaiveDate,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "date": from.to_string(),
            "totalExecutions": 1,
            "passedTestCases": 1,
            "failedTestCases": 1,
            "totalDurationSeconds": 60,
        })])
    }

    async fn build_comparison_analytics(
        &self,
        _user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "buildId": "build-1", "buildVersion": "v1", "deviceType": "Racer",
            "averageDuration": 60000, "totalTestCases": count.min(2),
            "passedPercentage": 50.0,
        })])
    }

    async fn build_comparison_analytics_by_id(
        &self,
        _user_id: &str,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((build_id == "build-1").then(|| {
            serde_json::json!({
                "buildId": build_id, "buildVersion": "v1", "deviceType": "Racer",
                "averageDuration": 60000, "totalTestCases": 2, "passedPercentage": 50.0,
            })
        }))
    }

    async fn build_performance_analytics(
        &self,
        _count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![build_performance_fixture("build-1")])
    }

    async fn build_performance_analytics_by_id(
        &self,
        build_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(build_performance_fixture(build_id))
    }

    async fn device_usage_analytics(
        &self,
        _start: chrono::DateTime<chrono::Utc>,
        _end: chrono::DateTime<chrono::Utc>,
        device_id: Option<&str>,
        device_family: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(serde_json::json!({
            "totalDevices": 1,
            "totalSeconds": 3600,
            "states": {"busy": 0, "not_reachable": 0, "free": 3600, "faulty": 0},
            "deviceId": device_id,
            "deviceFamily": device_family,
        }))
    }

    async fn test_execution_analytics(
        &self,
        _user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let counts = serde_json::json!({
            "total": 2, "passed": 1, "failed": 1, "inProgress": 0,
        });
        Ok(serde_json::json!({
            "allTime": counts,
            "weekly": [{"week": "28 Sep - 04 Oct", "total": 2,
                "passed": 1, "failed": 1, "inProgress": 0}],
            "monthly": [{"month": "Oct 2026", "total": 2,
                "passed": 1, "failed": 1, "inProgress": 0}],
        }))
    }

    async fn in_progress_test_analytics(
        &self,
        _user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "id": 1, "testPlanName": "Smoke", "createdAt": "2026-10-02T00:00:00Z",
            "startedAt": "2026-10-02T00:00:00Z", "endedAt": null,
            "status": "in_progress", "total": 2, "executed": 1,
        })])
    }

    async fn test_plan_analytics(
        &self,
        _user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(serde_json::json!({
            "total": 4, "completed": 1, "failed": 1,
            "inProgress": 1, "cancelled": 1,
        }))
    }

    async fn daily_test_analytics(
        &self,
        _user_id: &str,
        days: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "date": "2026-10-02", "totalExecutionSeconds": 60,
            "passed": days.min(1), "failed": 1,
        })])
    }

    async fn admin_alerts(
        &self,
        page: i64,
        _limit: i64,
        _sort_by: &str,
        _descending: bool,
    ) -> Result<AlertList, DeviceRepositoryError> {
        Ok(AlertList {
            data: vec![serde_json::json!({
                "id": "alert-1", "userId": "user-1", "title": "Device unavailable",
                "message": "Device device-1 is unavailable", "data": {"deviceId": "device-1"},
                "type": "warning", "status": "unread", "isRead": false,
                "readAt": null, "createdAt": "2026-10-02T00:00:00Z",
                "updatedAt": "2026-10-02T00:00:00Z", "deviceId": "device-1",
            })],
            total_data: 1,
            total_pages: 1,
            current_page: page,
            total_unread_count: None,
        })
    }

    async fn all_alerts(
        &self,
        page: i64,
        _limit: i64,
        _from: Option<chrono::DateTime<chrono::Utc>>,
        _to: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<AlertList, DeviceRepositoryError> {
        Ok(AlertList {
            data: vec![serde_json::json!({"id": "alert-1", "isRead": false})],
            total_data: 1,
            total_pages: 1,
            current_page: page,
            total_unread_count: Some(1),
        })
    }

    async fn mark_alert_read(&self, alert_id: &str) -> Result<bool, DeviceRepositoryError> {
        Ok(alert_id != "missing")
    }

    async fn mark_all_alerts_read(&self) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }

    async fn create_faulty_report(
        &self,
        request: &FaultyReportCreate,
    ) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError> {
        if request.release_id == "missing" {
            return Ok(None);
        }
        Ok(Some(FaultyReportCreation {
            last_test_execution_id: request.test_execution_id.clone(),
        }))
    }

    async fn finalize_faulty_report(
        &self,
        report_id: Uuid,
        file_path: Option<&str>,
        logs_path: Option<&str>,
    ) -> Result<repository_types::FaultyReportFinalization, DeviceRepositoryError> {
        Ok(repository_types::FaultyReportFinalization {
            report: serde_json::json!({
                "id": report_id,
                "releaseId": "11111111-1111-4111-8111-111111111111",
                "deviceType": "Racer",
                "deviceFamily": "Gen5",
                "description": "Intermittent failure",
                "filePath": file_path,
                "logsPath": logs_path,
                "status": "pending",
                "createdBy": "user-1",
            }),
            alert: serde_json::json!({
                "id": "alert-1",
                "userId": "ADMIN",
                "title": "Faulty Report",
                "message": "New faulty report submitted for release 11111111-1111-4111-8111-111111111111",
                "type": "warning",
                "status": "unread",
                "isRead": false,
                "faultyReportId": report_id,
                "buildId": "11111111-1111-4111-8111-111111111111",
                "buildVersion": "v1",
                "data": {"faultyReportId": report_id},
                "createdAt": "2026-10-02T12:00:00Z"
            }),
        })
    }

    async fn faulty_reports(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "id": "11111111-1111-4111-8111-111111111111",
            "status": "pending",
            "createdBy": "user-1",
        })])
    }

    async fn faulty_report_by_id(
        &self,
        report_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let file_path = self
            .callbacks
            .lock()
            .unwrap()
            .iter()
            .find_map(|value| value.strip_prefix("faulty-file:").map(str::to_owned));
        Ok((report_id != Uuid::nil()).then(|| {
            serde_json::json!({
                "report": {
                    "id": report_id,
                    "releaseId": "11111111-1111-4111-8111-111111111111",
                    "deviceType": "Racer",
                    "deviceFamily": "Gen5",
                    "description": "Intermittent failure",
                    "filePath": file_path,
                    "logsPath": null,
                    "status": "pending",
                    "createdBy": "user-1",
                },
                "buildVersion": "v1",
            })
        }))
    }

    async fn update_faulty_report_status(
        &self,
        report_id: Uuid,
        status: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((report_id != Uuid::nil()).then(|| {
            serde_json::json!({
                "id": report_id,
                "status": status,
            })
        }))
    }

    async fn delete_faulty_report(&self, report_id: Uuid) -> Result<bool, DeviceRepositoryError> {
        Ok(report_id != Uuid::nil())
    }

    async fn record_controller_heartbeat(
        &self,
        heartbeat: &ControllerHeartbeat,
    ) -> Result<ControllerHeartbeatResult, DeviceRepositoryError> {
        self.callbacks
            .lock()
            .unwrap()
            .push(format!("heartbeat:{}:{}", heartbeat.uid, heartbeat.ip));
        Ok(ControllerHeartbeatResult {
            state_changed: false,
            alert: None,
        })
    }

    async fn record_device_heartbeat(
        &self,
        device_id: &str,
        heartbeat: &serde_json::Value,
    ) -> Result<Option<DeviceHeartbeatResult>, DeviceRepositoryError> {
        self.callbacks
            .lock()
            .unwrap()
            .push(format!("device-heartbeat:{device_id}"));
        Ok(Some(DeviceHeartbeatResult {
            state: "free".to_owned(),
            state_changed: false,
            interface_changes: Vec::new(),
            heartbeat_data: heartbeat.get("data").cloned().unwrap_or_default(),
            alerts: Vec::new(),
            post_update_events: Vec::new(),
            deferred_cancellations: Vec::new(),
        }))
    }
}

#[test]
fn edgecontroller_state_map_becomes_channel_inventory() {
    let relay = RelayRegistration {
        serial_number: "relay-1".to_owned(),
        state: BTreeMap::from([("channel_1".to_owned(), 0), ("channel_8".to_owned(), 1)]),
        channels: Vec::new(),
    };
    let channels = validation::relay_channels(&relay).unwrap();
    assert_eq!(channels.len(), 2);
    assert_eq!(channels[0].channel_number, 1);
    assert_eq!(channels[1].channel_number, 8);
}

#[test]
fn controller_identity_normalizes_common_mac_formats() {
    assert_eq!(
        validation::normalized_controller_id("AA:bb:CC:dd:EE:ff").unwrap(),
        "aabbccddeeff"
    );
    assert_eq!(
        validation::normalized_controller_id("AA-BB-CC-DD-EE-FF").unwrap(),
        "aabbccddeeff"
    );
}

#[tokio::test]
async fn edgecontroller_registration_is_create_then_idempotent_update() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let payload = serde_json::json!({
        "macAddress": "AA:BB:CC:DD:EE:FF",
        "ipAddress": "192.0.2.10",
        "deviceFamily": "Gen5",
        "relays": [{
            "serialNumber": "relay-1",
            "state": {"channel_0": 0, "channel_1": 1}
        }],
        "uid": "aabbccddeeff"
    });
    for (path, expected) in [
        ("/api/v1/device/controller/", StatusCode::CREATED),
        ("/api/v1/device/controller", StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["success"], true);
        assert_eq!(body["data"]["deviceControllerId"], "aabbccddeeff");
        let event = events.recv().await.unwrap();
        assert_eq!(event.event, "device_controller_changed");
        assert_eq!(
            event.payload["action"],
            if expected == StatusCode::CREATED {
                "added"
            } else {
                "updated"
            }
        );
        assert_eq!(event.payload["controllerId"], "aabbccddeeff");
        assert!(event.room.is_none());
        if expected == StatusCode::CREATED {
            let alert = events.recv().await.unwrap();
            assert_eq!(alert.event, "alert");
            assert_eq!(alert.payload["subtype"], "device-controller-addition");
        }
    }
}

#[tokio::test]
async fn registration_request_publishes_pending_user_frontend_event() {
    let mut config = AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap();
    config.modules.enabled.insert(crate::config::Module::Auth);
    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    );
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let response = app
        .oneshot(
            Request::post("/api/v1/auth/register/request")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "username": "pending.user",
                        "email": "pending@example.com",
                        "firstName": "Pending",
                        "lastName": "User"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let event = events.recv().await.unwrap();
    assert_eq!(event.event, "user");
    assert!(event.room.is_none());
    assert_eq!(event.payload["message"]["id"], "pending-user-1");
    assert_eq!(event.payload["message"]["username"], "pending.user");
    assert_eq!(event.payload["message"]["email"], "pending@example.com");
    assert_eq!(
        event.payload["message"]["requiredActions"][0],
        "UPDATE_PASSWORD"
    );
    assert!(event.payload["timestamp"].is_string());
}

#[tokio::test]
async fn device_registration_is_public_and_preserves_wire_contract() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let response = app
        .oneshot(
            Request::post("/api/v1/device/")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "macAddress": "00:1A:2B:3C:4D:5E",
                        "ipAddress": "192.0.2.20",
                        "deviceName": "Bench device",
                        "interfaces": {"ethernet": ["eth0"]}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["deviceId"], "001a2b3c4d5e");
    assert_eq!(body["data"]["heartbeatTimer"], 5);
    assert_eq!(body["data"]["interfaces"][0]["type"], "ethernet");
    for subtype in ["device-addition", "device-approval"] {
        let event = events.recv().await.unwrap();
        assert_eq!(event.event, "alert");
        assert_eq!(event.payload["type"], "alert");
        assert_eq!(event.payload["subtype"], subtype);
        assert_eq!(event.payload["message"]["deviceId"], "001a2b3c4d5e");
        assert!(event.payload["timestamp"].is_string());
    }
}

#[tokio::test]
async fn device_registration_maps_invalid_and_missing_fields_to_bad_request() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }));
    let app = api::router(Arc::new(state));

    for payload in [
        serde_json::json!({
            "macAddress": "invalid",
            "ipAddress": "192.0.2.20",
            "deviceName": "Bench device"
        }),
        serde_json::json!({
            "macAddress": "00:1A:2B:3C:4D:5E",
            "deviceName": "Bench device"
        }),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/device/")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn device_csv_export_requires_admin_and_returns_attachment() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    config.device.build_upload_dir = directory.path().to_string_lossy().into_owned();
    let unauthenticated = api::router(Arc::new(
        AppState::without_dependencies(config.clone(), Metrics::new().unwrap())
            .with_device_repository(repository.clone()),
    ));
    let response = unauthenticated
        .oneshot(
            Request::get("/api/v1/device/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(repository);
    let authenticated = api::router(Arc::new(state));
    let response = authenticated
        .clone()
        .oneshot(
            Request::get("/api/v1/device/export?status=declined&search=Bench")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/csv");
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment; filename=\"devices-")
    );
    let body = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("deviceId,deviceName,deviceType"));
    assert!(body.contains("ethernet_interfaces"));
    assert!(body.contains("device-1,Bench 1,x5h"));
}

#[tokio::test]
async fn flashing_callback_is_public_and_reports_missing_devices() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    for (device_id, expected) in [
        ("device-1", StatusCode::OK),
        ("missing", StatusCode::NOT_FOUND),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/device/{device_id}/flashing"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        if expected == StatusCode::OK {
            assert_eq!(body["success"], true);
            assert_eq!(
                body["message"],
                "Device device-1 is now marked as upgrading"
            );
            let event = events.recv().await.unwrap();
            assert_eq!(event.event, "device_state_update");
            assert_eq!(event.payload["deviceId"], "device-1");
            assert_eq!(event.payload["state"], "busy");
            assert_eq!(event.payload["upgrading"], true);
            assert_eq!(event.payload["flashing"], false);
            assert!(event.payload["changedAt"].is_string());
        }
    }
}

#[tokio::test]
async fn test_completion_callback_sanitizes_ids_and_reports_errors() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }));
    let app = api::router(Arc::new(state));
    for (query, expected) in [
        ("testId=%20%22test-1%27%20", StatusCode::OK),
        ("testId=missing", StatusCode::NOT_FOUND),
        ("testId=%20%27%22%20", StatusCode::BAD_REQUEST),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/device/device-1/test-completed?{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body["success"], true);
            assert_eq!(
                body["message"],
                "Test completion handled for device device-1"
            );
        }
    }
}

#[tokio::test]
async fn build_list_requires_auth_and_preserves_frontend_shape() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let config = AppConfig::load(&cli).unwrap();
    let unauthenticated = api::router(Arc::new(
        AppState::without_dependencies(config.clone(), Metrics::new().unwrap())
            .with_device_repository(repository.clone()),
    ));
    let response = unauthenticated
        .oneshot(
            Request::get("/api/v1/device/build")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(repository);
    let mut events = state.event_publisher.subscribe();
    let authenticated = api::router(Arc::new(state));
    let response = authenticated
        .clone()
        .oneshot(
            Request::get("/api/v1/device/build?page=0&limit=20&flagged=true")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["builds"][0]["id"], "build-1");
    assert_eq!(body["totalCount"], 1);
    assert_eq!(body["currentPage"], 1);
    assert_eq!(body["totalPages"], 1);
    assert_eq!(body["requestedCount"], 1);

    for (build_id, expected) in [
        ("build-1", StatusCode::OK),
        ("missing", StatusCode::NOT_FOUND),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/device/build/{build_id}"))
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body["id"], "build-1");
            assert_eq!(body["version"], "1.2.3");
        }
    }

    let response = authenticated
        .clone()
        .oneshot(
            Request::get("/api/v1/device/build/filters")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["deviceTypes"], serde_json::json!(["x5h"]));
    assert_eq!(body["deviceFamilies"], serde_json::json!(["Gen5"]));

    for (payload, expected) in [
        (serde_json::json!({"fileCount": 3}), StatusCode::CREATED),
        (serde_json::json!({"fileCount": 0}), StatusCode::BAD_REQUEST),
        (serde_json::json!({}), StatusCode::BAD_REQUEST),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::post("/api/v1/device/build/upload/init")
                    .header("authorization", "Bearer access")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::CREATED {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body["uploadId"], Uuid::nil().to_string());
        }
    }

    let multipart = |include_tag: bool| {
        let tag = if include_tag {
            "--boundary\r\nContent-Disposition: form-data; name=\"tag\"\r\n\r\nnightly\r\n"
        } else {
            ""
        };
        format!(
            "--boundary\r\nContent-Disposition: form-data; name=\"uploadId\"\r\n\r\n{}\r\n{tag}--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x5h__1.2.3.zip\"\r\nContent-Type: application/zip\r\n\r\nzip\r\n--boundary--\r\n",
            Uuid::nil()
        )
    };
    for (path, include_tag, expected) in [
        ("/api/v1/device/build/upload/custom", true, StatusCode::OK),
        (
            "/api/v1/device/build/upload/custom",
            false,
            StatusCode::BAD_REQUEST,
        ),
        ("/api/v1/device/build/upload", false, StatusCode::OK),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::post(path)
                    .header("authorization", "Bearer access")
                    .header("content-type", "multipart/form-data; boundary=boundary")
                    .body(Body::from(multipart(include_tag)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body["message"], "Upload successful");
            assert_eq!(body["filename"], "x5h__1.2.3.zip");
            let progress = events.recv().await.unwrap();
            assert_eq!(progress.event, "upload_progress");
            assert_eq!(progress.payload["percent"], 0);
            assert_eq!(progress.room.as_deref(), Some("user:system"));
            let progress = events.recv().await.unwrap();
            assert_eq!(progress.event, "upload_progress");
            assert_eq!(progress.payload["percent"], 100);
            let complete = events.recv().await.unwrap();
            assert_eq!(complete.event, "upload_complete");
            assert_eq!(complete.payload["uploadId"], Uuid::nil().to_string());
            assert_eq!(complete.payload["fileName"], "x5h__1.2.3.zip");
            let uploaded = events.recv().await.unwrap();
            assert_eq!(uploaded.event, "build_uploaded");
            assert_eq!(uploaded.payload["deviceType"], "x5h");
            let alert = events.recv().await.unwrap();
            assert_eq!(alert.event, "alert");
            assert_eq!(alert.payload["message"]["title"], "Build Upload Success");
        }
    }

    for (build_id, payload, expected) in [
        (
            "build-1",
            serde_json::json!({"isFaulty": true}),
            StatusCode::OK,
        ),
        ("build-1", serde_json::json!({}), StatusCode::BAD_REQUEST),
        (
            "missing",
            serde_json::json!({"isFaulty": false}),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::put(format!("/api/v1/device/build/{build_id}/flag"))
                    .header("authorization", "Bearer access")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body, serde_json::json!({"id": "build-1", "isFaulty": true}));
            let event = events.recv().await.unwrap();
            assert_eq!(event.event, "build_flagged");
            assert_eq!(event.payload["releaseId"], "build-1");
            assert_eq!(event.payload["version"], "v1.2.3");
            let alert = events.recv().await.unwrap();
            assert_eq!(alert.event, "alert");
            assert_eq!(alert.payload["message"]["title"], "Build Flagged");
        }
    }

    for (build_id, expected) in [
        ("build-1", StatusCode::NO_CONTENT),
        ("missing", StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::delete(format!("/api/v1/device/build/{build_id}"))
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::NO_CONTENT {
            let event = events.recv().await.unwrap();
            assert_eq!(event.event, "build_deleted");
            assert_eq!(event.payload["deviceType"], "x5h");
            let alert = events.recv().await.unwrap();
            assert_eq!(alert.event, "alert");
            assert_eq!(alert.payload["message"]["title"], "Build Deleted");
        }
    }
}

#[tokio::test]
async fn tus_upload_preserves_offsets_and_protocol_headers() {
    let directory = tempfile::tempdir().unwrap();
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    config.device.build_upload_dir = directory.path().to_string_lossy().into_owned();
    config.device.build_upload_max_bytes = 16;
    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/build/upload/tus")
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .header("upload-length", "5")
                .header(
                    "upload-metadata",
                    "filename YnVpbGQuYmlu,uploadId YmF0Y2gtMQ==",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["tus-resumable"], "1.0.0");
    let location = response.headers()["location"].to_str().unwrap().to_owned();
    assert!(location.starts_with("/api/v1/device/build/upload/tus/"));

    let response = app
        .clone()
        .oneshot(
            Request::head(&location)
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["upload-offset"], "0");
    assert_eq!(response.headers()["upload-length"], "5");

    let wrong_offset = app
        .clone()
        .oneshot(
            Request::patch(&location)
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .header("content-type", "application/offset+octet-stream")
                .header("upload-offset", "2")
                .body(Body::from("abc"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_offset.status(), StatusCode::CONFLICT);

    let response = app
        .clone()
        .oneshot(
            Request::patch(&location)
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .header("content-type", "application/offset+octet-stream")
                .header("upload-offset", "0")
                .body(Body::from("abc"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(response.headers()["upload-offset"], "3");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/device/build/upload/tus")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["tus-version"], "1.0.0");

    let create_final = |metadata: &'static str| {
        Request::post("/api/v1/device/build/upload/tus")
            .header("authorization", "Bearer access")
            .header("x-user-id", "user-1")
            .header("tus-resumable", "1.0.0")
            .header("upload-length", "3")
            .header("upload-metadata", metadata)
            .body(Body::empty())
            .unwrap()
    };
    let response = app
        .clone()
        .oneshot(create_final(
            "filename eDVoX18xLjIuMy56aXA=,uploadId MDAwMDAwMDAtMDAwMC0wMDAwLTAwMDAtMDAwMDAwMDAwMDAw",
        ))
        .await
        .unwrap();
    let complete_location = response.headers()["location"].to_str().unwrap().to_owned();
    let response = app
        .clone()
        .oneshot(
            Request::patch(&complete_location)
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .header("content-type", "application/offset+octet-stream")
                .header("upload-offset", "0")
                .body(Body::from("zip"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(response.headers()["upload-offset"], "3");

    for (event, percent) in [
        ("upload_progress", Some(0)),
        ("upload_progress", Some(60)),
        ("upload_progress", Some(0)),
        ("upload_progress", Some(100)),
        ("upload_complete", None),
        ("build_uploaded", None),
        ("alert", None),
    ] {
        let received = events.recv().await.unwrap();
        assert_eq!(received.event, event);
        if let Some(percent) = percent {
            assert_eq!(received.payload["percent"], percent);
            assert_eq!(received.room.as_deref(), Some("user:user-1"));
        }
    }

    let response = app
        .clone()
        .oneshot(create_final("filename eDVoX18xLjIuMy56aXA="))
        .await
        .unwrap();
    let invalid_location = response.headers()["location"].to_str().unwrap().to_owned();
    let response = app
        .clone()
        .oneshot(
            Request::patch(&invalid_location)
                .header("authorization", "Bearer access")
                .header("tus-resumable", "1.0.0")
                .header("content-type", "application/offset+octet-stream")
                .header("upload-offset", "0")
                .body(Body::from("zip"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = app
        .clone()
        .oneshot(
            Request::head(&invalid_location)
                .header("authorization", "Bearer access")
                .header("tus-resumable", "1.0.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["upload-offset"], "0");

    let response = app
        .clone()
        .oneshot(create_final(
            "filename ZmFpbC56aXA=,uploadId MDAwMDAwMDAtMDAwMC0wMDAwLTAwMDAtMDAwMDAwMDAwMDAw",
        ))
        .await
        .unwrap();
    let failed_location = response.headers()["location"].to_str().unwrap().to_owned();
    let response = app
        .clone()
        .oneshot(
            Request::patch(&failed_location)
                .header("authorization", "Bearer access")
                .header("x-user-id", "user-1")
                .header("tus-resumable", "1.0.0")
                .header("content-type", "application/offset+octet-stream")
                .header("upload-offset", "0")
                .body(Body::from("zip"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    for (event, percent) in [
        ("upload_progress", Some(0)),
        ("upload_progress", Some(100)),
        ("upload_error", None),
    ] {
        let received = events.recv().await.unwrap();
        assert_eq!(received.event, event);
        if let Some(percent) = percent {
            assert_eq!(received.payload["percent"], percent);
        } else {
            assert_eq!(received.payload["message"], "upload processing failed");
            assert_eq!(received.room.as_deref(), Some("user:user-1"));
        }
    }
    let response = app
        .oneshot(
            Request::head(&failed_location)
                .header("authorization", "Bearer access")
                .header("tus-resumable", "1.0.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["upload-offset"], "0");
}

#[tokio::test]
async fn test_export_requires_auth_and_returns_csv_attachment() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let config = AppConfig::load(&cli).unwrap();
    let unauthenticated = api::router(Arc::new(
        AppState::without_dependencies(config.clone(), Metrics::new().unwrap())
            .with_device_repository(repository.clone()),
    ));
    let response = unauthenticated
        .oneshot(
            Request::get("/api/v1/device/test/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            config,
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(repository),
    ));
    let response = app
        .oneshot(
            Request::get("/api/v1/device/test/export")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/csv");
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment; filename=\"test-executions-")
    );
    let body = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.starts_with("S.No,Test Name,Status,Device,Duration"));
    assert!(body.contains("1,Smoke,Completed,x5h,1 Min,1,1,0,,100"));
}

#[tokio::test]
async fn test_plan_route_requires_auth_and_returns_bare_array() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let config = AppConfig::load(&cli).unwrap();
    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    }))
    .with_test_catalog(Arc::new(FakeTestCatalog));
    let state = state.with_qmetry_catalog(Arc::new(FakeQmetryCatalog));
    let app = api::router(Arc::new(state));
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/plan?filter=Smoke&buildId=b1&deviceId=d1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["id"], 7);
    assert_eq!(body[0]["name"], "Smoke");
    assert_eq!(body[0]["testSuits"], serde_json::json!([]));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/suite?planId=7")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["id"], 8);
    assert_eq!(body[0]["planId"], 7);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/suite")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/testcase?planId=7&suiteId=8&filter=Boot")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["id"], 9);
    assert_eq!(body[0]["scriptFile"], "run.sh");
    assert_eq!(body[0]["labels"], serde_json::json!(["smoke"]));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/testcase?planId=7")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/plan/7/testcase")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["8"][0]["labels"], serde_json::json!(["smoke"]));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/plan/7/suite/8/testcase")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["id"], "case-1");

    let response = app
        .oneshot(
            Request::get("/api/v1/device/test/plan/1/testcase")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_execution_detail_supports_latest_and_configured_logs() {
    let directory = tempfile::tempdir().unwrap();
    tokio::fs::write(directory.path().join("test-1.json"), br#"["started"]"#)
        .await
        .unwrap();
    let report_directory = directory.path().join("x5h/v1.2.3/test-1");
    tokio::fs::create_dir_all(&report_directory).await.unwrap();
    tokio::fs::write(report_directory.join("report.html"), "<h1>Report</h1>")
        .await
        .unwrap();
    tokio::fs::write(report_directory.join("case.log"), "case output")
        .await
        .unwrap();
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    config.tests.test_logs_dir = directory.path().to_string_lossy().into_owned();
    config.reports.test_results_dir = directory.path().to_path_buf();
    let metrics = Metrics::new().unwrap();
    let reports = ReportService::new(
        config.reports.clone(),
        Arc::new(FakeReportDataSource),
        metrics.clone(),
    );
    let app = api::router(Arc::new(
        AppState::with_identity_provider(config, metrics, Arc::new(AcceptingIdentityProvider))
            .with_device_repository(Arc::new(RegistrationRepository {
                calls: AtomicUsize::new(0),
                callbacks: Mutex::new(Vec::new()),
            }))
            .with_reports(reports),
    ));
    for (id, expected) in [
        ("test-1", StatusCode::OK),
        ("latest", StatusCode::OK),
        ("missing", StatusCode::NOT_FOUND),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/device/test/execution/{id}"))
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body["testId"], "test-1");
            assert_eq!(body["logs"], serde_json::json!(["started"]));
        }
    }
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/test-1?table=true")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["testId"], "test-1");
    assert_eq!(body["durationSeconds"], 60);
    assert_eq!(body["total"], 1);
    assert_eq!(body["testCases"][0]["testCaseId"], 11);
    assert_eq!(body["analytics"]["allTime"]["passed"], 1);
    assert_eq!(body["logs"], serde_json::json!(["started"]));

    for (test_id, expected) in [
        ("test-1", StatusCode::OK),
        ("missing", StatusCode::NOT_FOUND),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/device/test/execution/single/{test_id}"))
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body["testCases"][0]["testCaseId"], 11);
        }
    }
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/list/test-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body, serde_json::json!(["11", "12"]));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/device/device-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["logs"], serde_json::json!(["started"]));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/progress/list")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["deviceId"], "device-1");
    assert_eq!(body[0]["status"], "in_progress");

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution?page=0&limit=20&sortBy=deviceType&desc=false&search=x5")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["currentPage"], 1);
    assert_eq!(body["totalPages"], 1);
    assert_eq!(body["data"][0]["testId"], "test-1");
    assert_eq!(body["data"][0]["testCases"], serde_json::json!({}));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/cases/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["testCaseId"], 11);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/testcase/1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/logs/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/plain");
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"test-test-1-logs.txt\""
    );
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "started"
    );

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/report/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/report/test-1/html")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "<h1>Report</h1>"
    );

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/testcase/1/log")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"case.log\""
    );
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "case output"
    );

    let response = app
        .clone()
        .oneshot(
            Request::put("/api/v1/device/test/execution/report/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["testExecutionId"], "test-1");
    assert_eq!(body["status"], "generating");

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/build/build-1?limit=2&offset=1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["limit"], 2);
    assert_eq!(body["offset"], 1);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/test/execution/results/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::put("/api/v1/device/test/cancel/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["message"], "  Test execution cancelled successfully");
}

#[tokio::test]
async fn edgecontroller_callbacks_preserve_wire_contracts() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state =
        AppState::without_dependencies(AppConfig::load(&cli).unwrap(), Metrics::new().unwrap())
            .with_device_repository(repository.clone());
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));

    let mapping = serde_json::json!({
        "mac": "AA:BB:CC:DD:EE:FF",
        "ip": "192.0.2.10",
        "status": "success",
        "tty_entry": "{\"uart\":\"/dev/ttyUSB0\",\"power\":\"/dev/ttyUSB1\"}"
    });
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/mapping-gen5")
                .header("content-type", "application/json")
                .body(Body::from(mapping.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    for path in [
        "/api/v1/device/flash-confirm?status=success",
        "/api/v1/device/flash-confirm-gen4?status=failure",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    assert_eq!(
        *repository.callbacks.lock().unwrap(),
        [
            "mapping:192.0.2.10:AA:BB:CC:DD:EE:FF:Success:/dev/ttyUSB0",
            "flash:x5h:Success",
            "flash:v4h:Failure",
        ]
    );
    let cancellation = events.recv().await.unwrap();
    assert_eq!(cancellation.event, "test_execution_update");
    assert_eq!(cancellation.payload["testId"], "test-1");
    assert_eq!(
        cancellation.payload["update"]["data"]["status"],
        "cancelled"
    );
    assert_eq!(cancellation.room.as_deref(), Some("user:user-1"));
    let device = events.recv().await.unwrap();
    assert_eq!(device.event, "device_state_update");
    assert_eq!(device.payload["deviceId"], "device-1");
    assert_eq!(device.payload["state"], "free");
}

#[tokio::test]
async fn frontend_family_and_type_reads_require_auth_and_return_bare_arrays() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let config = AppConfig::load(&cli).unwrap();
    let unauthenticated = api::router(Arc::new(
        AppState::without_dependencies(config.clone(), Metrics::new().unwrap())
            .with_device_repository(repository.clone()),
    ));
    let response = unauthenticated
        .oneshot(
            Request::get("/api/v1/device/families")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let authenticated = api::router(Arc::new(
        AppState::with_identity_provider(
            config,
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(repository),
    ));
    for (path, expected) in [
        (
            "/api/v1/device/families",
            serde_json::json!(["Gen4", "Gen5"]),
        ),
        (
            "/api/v1/device/deviceTypes?deviceFamily=Gen5",
            serde_json::json!(["x5h"]),
        ),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::get(path)
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body, expected);
    }

    for (path, expected_data) in [
        (
            "/api/v1/device/device-1",
            serde_json::json!({
                "deviceId": "device-1",
                "deviceFamily": "Gen5",
                "interfaces": [],
                "testExecutions": []
            }),
        ),
        (
            "/api/v1/device/device-1/heartbeat",
            serde_json::json!({
                "timestamp": "2026-10-02T00:00:00Z",
                "data": {"cpuUsagePercent": 12.5},
                "timeout": 30
            }),
        ),
    ] {
        let response = authenticated
            .clone()
            .oneshot(
                Request::get(path)
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["success"], true);
        assert_eq!(body["data"], expected_data);
    }

    let response = authenticated
        .oneshot(
            Request::get("/api/v1/device/missing")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ))
    .oneshot(
        Request::get("/api/v1/device/topology")
            .header("authorization", "Bearer access")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"][0]["controllerId"], "aabbccddeeff");
}

#[tokio::test]
async fn relay_configuration_preserves_admin_and_callback_contracts() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state = AppState::with_identity_provider(
        AppConfig::load(&cli).unwrap(),
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(repository);
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let relay_id = Uuid::new_v4();
    let channel_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "relayId": relay_id,
        "channelId": channel_id,
        "deviceId": "device-1",
        "gpio": "17",
        "gpioDefaultLevel": "LOW",
        "relayDefaultLevel": "LOW"
    });

    let unauthorized = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/relay/configure")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let accepted = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/relay/configure")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let body: serde_json::Value =
        serde_json::from_slice(&accepted.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"]["id"], channel_id.to_string());

    let confirmation = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/relay/config/confirmation")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"mac":"AA:BB","status":"success"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(confirmation.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&confirmation.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(body["confirmed"], true);
    let power_event = events.recv().await.unwrap();
    assert_eq!(power_event.event, "device_power_update");
    assert_eq!(power_event.payload["deviceId"], "device-1");
    let status_event = events.recv().await.unwrap();
    assert_eq!(status_event.event, "relay_configuration_status");
    assert_eq!(status_event.payload["status"], "completed");
    assert_eq!(status_event.payload["completed"], 1);
    let controller_event = events.recv().await.unwrap();
    assert_eq!(controller_event.event, "device_controller_changed");
    assert_eq!(controller_event.payload["controllerId"], "controller-1");

    let invalid = app
        .oneshot(
            Request::post("/api/v1/device/relay/config/confirmation")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"mac":"AA:BB","status":"unknown"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn device_power_toggle_preserves_frontend_http_and_event_contracts() {
    let _controller_guard = edge_controller_test_lock().lock().await;
    let controller_requests = Arc::new(Mutex::new(Vec::new()));
    let captured_requests = Arc::clone(&controller_requests);
    let controller = axum::Router::new().route(
        "/relay",
        axum::routing::post(move |Json(payload): Json<serde_json::Value>| {
            let captured_requests = Arc::clone(&captured_requests);
            async move {
                captured_requests.lock().unwrap().push(payload);
                StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", EDGE_CONTROLLER_PORT))
        .await
        .unwrap();
    let controller_task = tokio::spawn(async move {
        axum::serve(listener, controller).await.unwrap();
    });

    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let state = AppState::with_identity_provider(
        AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap(),
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(repository.clone());
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));

    let response = app
        .oneshot(
            Request::put("/api/v1/device/relay/toggle/power-device")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["deviceId"], "power-device");
    assert_eq!(body["channelNumber"], 2);
    assert_eq!(body["state"], "off");
    assert_eq!(
        controller_requests.lock().unwrap().as_slice(),
        &[serde_json::json!({"serial": "relay-1", "state": "off", "channel": 2})]
    );
    assert_eq!(
        repository.callbacks.lock().unwrap().as_slice(),
        &["power:power-device:off"]
    );
    let event = events.recv().await.unwrap();
    assert_eq!(event.event, "device_power_update");
    assert_eq!(event.payload["deviceId"], "power-device");
    assert_eq!(event.payload["power"], "off");
    assert!(event.payload["changedAt"].is_string());
    assert!(event.room.is_none());

    controller_task.abort();
}

#[tokio::test]
async fn heartbeat_timeout_preserves_frontend_and_device_callback_contracts() {
    let _controller_guard = edge_controller_test_lock().lock().await;
    let controller_requests = Arc::new(Mutex::new(Vec::new()));
    let captured_requests = Arc::clone(&controller_requests);
    let controller = axum::Router::new().route(
        "/configure/heartbeat",
        axum::routing::post(move |Json(payload): Json<serde_json::Value>| {
            let captured_requests = Arc::clone(&captured_requests);
            async move {
                captured_requests.lock().unwrap().push(payload);
                StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", EDGE_CONTROLLER_PORT))
        .await
        .unwrap();
    let controller_task = tokio::spawn(async move {
        axum::serve(listener, controller).await.unwrap();
    });

    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(repository.clone()),
    ));
    let response = app
        .oneshot(
            Request::put("/api/v1/device/heartbeat/timeout")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"deviceId": "heartbeat-device", "value": 15}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["message"], "Heartbeat timeout saved successfully.");
    assert_eq!(
        controller_requests.lock().unwrap().as_slice(),
        &[serde_json::json!({"timeout": 15})]
    );
    assert_eq!(
        repository.callbacks.lock().unwrap().as_slice(),
        &["heartbeat:heartbeat-device:15"]
    );

    controller_task.abort();
}

#[tokio::test]
async fn available_relay_devices_preserves_frontend_picker_contract() {
    let repository = Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    });
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(repository.clone()),
    ));
    let relay_id = Uuid::new_v4();

    let unauthorized = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/relay/devices/available")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::get(format!(
                "/api/v1/device/relay/devices/available?page=2&limit=5&relayId={relay_id}"
            ))
            .header("authorization", "Bearer access")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["devices"][0]["deviceId"], "device-1");
    assert_eq!(body["devices"][0]["deviceType"], "Racer");
    assert_eq!(body["devices"][0]["macAddress"], "00:11:22:33:44:55");
    assert_eq!(body["pagination"]["page"], 2);
    assert_eq!(body["pagination"]["limit"], 5);
    assert_eq!(body["pagination"]["totalCount"], 1);
    assert_eq!(body["pagination"]["totalPages"], 1);
    assert_eq!(
        repository.callbacks.lock().unwrap().as_slice(),
        &[format!("available:2:5:{relay_id}")]
    );
}

#[tokio::test]
async fn retained_relay_routes_preserve_legacy_contracts() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&cli).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ));
    let relay_id = Uuid::new_v4();
    let channel_id = Uuid::new_v4();

    let unauthorized = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/relay")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let configured = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/relay/config")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "id": channel_id,
                        "relayId": relay_id,
                        "deviceId": "device-1",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(configured.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&configured.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(body["data"]["id"], channel_id.to_string());
    assert_eq!(body["message"], "Device configured with relay successfully");

    for (path, payload) in [
        (
            "/api/v1/device/relay/config/fresh",
            serde_json::json!({"relayId": relay_id}),
        ),
        (
            "/api/v1/device/relay/config/remap",
            serde_json::json!({"relayId": relay_id}),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("authorization", "Bearer access")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let conflicts = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/device/relay/check-conflicts?relayId={relay_id}&channelNumber=1&deviceId=device-1"
            ))
            .header("authorization", "Bearer access")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflicts.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&conflicts.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["conflict"]["deviceConflict"], true);
    assert_eq!(body["conflict"]["channelConflict"], true);

    for path in [
        "/api/v1/device/relay?controllerId=controller-1".to_owned(),
        format!("/api/v1/device/relay/channel?relayId={relay_id}"),
        format!("/api/v1/device/relay/channel/{channel_id}"),
        format!("/api/v1/device/relay/{relay_id}"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(path)
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    for path in [
        format!("/api/v1/device/relay/channel/{channel_id}"),
        format!("/api/v1/device/relay/{relay_id}"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::delete(path)
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}

#[tokio::test]
async fn device_state_analytics_preserve_auth_and_grouping_contracts() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&cli).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ));

    let unauthorized = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/state")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let state = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/state")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(state.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&state.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["total"], 2);
    assert_eq!(body["stateCount"]["busy"], 1);
    assert_eq!(body["stateCount"]["free"], 1);

    let detailed = app
        .oneshot(
            Request::get("/api/v1/device/analytics/state/detailed")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detailed.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&detailed.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["Gen5"][0]["deviceId"], "device-1");
    assert_eq!(body["Gen5"][0]["deviceType"], "Racer");
    assert_eq!(body["Gen5"][0]["state"], "free");
}

#[tokio::test]
async fn execution_activity_analytics_preserve_legacy_shapes() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&cli).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ));

    let recent = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/execution?count=2")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(recent.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&recent.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["testId"], "test-1");
    assert_eq!(body[0]["totalTestCases"], 2);

    let daily = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/execution/daily?from=2026-10-01&to=2026-10-02")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(daily.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&daily.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["date"], "2026-10-01");
    assert_eq!(body[0]["totalDurationSeconds"], 60);

    let execution = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/execution/test-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(execution.status(), StatusCode::OK);

    let missing = app
        .oneshot(
            Request::get("/api/v1/device/analytics/execution/missing")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value =
        serde_json::from_slice(&missing.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["message"], "Test execution not found");
}

#[tokio::test]
async fn build_analytics_preserve_comparison_and_performance_shapes() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&cli).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ));

    for path in [
        "/api/v1/device/analytics/builds/comparison?count=2",
        "/api/v1/device/analytics/builds/performance?count=2",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(path)
                    .header("authorization", "Bearer access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body[0]["buildId"], "build-1");
    }

    let comparison = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/builds/comparison/build-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(comparison.status(), StatusCode::OK);

    let missing = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/builds/comparison/missing")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let performance = app
        .oneshot(
            Request::get("/api/v1/device/analytics/builds/performance/build-1")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(performance.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&performance.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(body["passedTestCasePercentage"], 50.0);
    assert_eq!(body["uniqueDeviceCount"], 1);
}

#[tokio::test]
async fn device_usage_analytics_preserve_daily_and_period_shapes() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&cli).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ));

    let daily = app
        .clone()
        .oneshot(
            Request::get(
                "/api/v1/device/analytics/usage/daily?day=2026-10-02&deviceId=device-1&deviceFamily=Gen5",
            )
            .header("authorization", "Bearer access")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(daily.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&daily.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["totalDevices"], 1);
    assert_eq!(body["totalSeconds"], 3600);
    assert_eq!(body["states"]["free"], 3600);

    let invalid = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/usage/daily?day=10-02-2026")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let summary = app
        .oneshot(
            Request::get("/api/v1/device/analytics/usage/summary")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(summary.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&summary.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["weekly"].as_array().unwrap().len(), 4);
    assert_eq!(body["monthly"].as_array().unwrap().len(), 4);
    assert!(body["weekly"][0]["weekStart"].is_string());
    assert!(body["monthly"][3]["monthEnd"].is_string());
}

#[tokio::test]
async fn test_analytics_preserve_type_plan_and_daily_contracts() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&cli).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ));

    let aggregate = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/test?type=execution")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(aggregate.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&aggregate.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["allTime"]["total"], 2);
    assert!(body["weekly"].is_array());
    assert!(body["monthly"].is_array());

    let in_progress = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/test?type=inProgress")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(in_progress.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&in_progress.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(body[0]["executed"], 1);

    let invalid = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/test?type=unknown")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let plan = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/analytics/test/plan")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(plan.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&plan.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["total"], 4);
    assert_eq!(body["cancelled"], 1);

    let daily = app
        .oneshot(
            Request::get("/api/v1/device/analytics/test/daily?days=8")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(daily.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&daily.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body[0]["totalExecutionSeconds"], 60);
}

#[tokio::test]
async fn notification_routes_preserve_visibility_and_read_contracts() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let app = api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&cli).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ));

    let unauthorized = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/notification/alerts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let admin = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/notification/alerts?page=2&limit=5")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&admin.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["currentPage"], 2);
    assert_eq!(body["data"][0]["deviceId"], "device-1");
    assert!(body.get("totalUnreadCount").is_none());

    let all = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/notification/alerts/all?from=invalid")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(all.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&all.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["totalUnreadCount"], 1);

    let missing = app
        .clone()
        .oneshot(
            Request::put("/api/v1/device/notification/alerts/missing/read")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let legacy = app
        .clone()
        .oneshot(
            Request::put("/api/v1/device/notification/alerts/read/missing")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::OK);

    let all_read = app
        .oneshot(
            Request::put("/api/v1/device/notification/alerts/all/read")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(all_read.status(), StatusCode::OK);
}

#[tokio::test]
async fn faulty_report_routes_preserve_create_admin_and_status_contracts() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let attachment = temporary.path().join("attachment.txt");
    std::fs::write(&attachment, "fault details").unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    config.device.faulty_report_upload_dir = temporary.path().display().to_string();
    let state = AppState::with_identity_provider(
        config,
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(vec![format!("faulty-file:{}", attachment.display())]),
    }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let token = "Bearer e30.eyJzdWIiOiJ1c2VyLTEifQ.e30";
    let boundary = "faulty-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"deviceType\"\r\n\r\nRacer\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"deviceFamily\"\r\n\r\nGen5\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"releaseId\"\r\n\r\n11111111-1111-4111-8111-111111111111\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"description\"\r\n\r\nIntermittent failure\r\n--{boundary}--\r\n"
    );
    let created = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/faulty/report")
                .header("authorization", token)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let event = events.recv().await.unwrap();
    assert_eq!(event.event, "alert");
    assert_eq!(event.payload["subtype"], "device-alert");
    assert_eq!(event.payload["message"]["title"], "Faulty Report");
    assert_eq!(event.payload["message"]["type"], "warning");
    assert_eq!(event.payload["message"]["buildVersion"], "v1");

    let list = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/faulty/report")
                .header("authorization", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);

    let id = "11111111-1111-4111-8111-111111111111";
    let detail = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/device/faulty/report/{id}"))
                .header("authorization", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&detail.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["buildVersion"], "v1");
    assert_eq!(body["user"]["userName"], "farm.user");

    let file = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/device/faulty/report/{id}/file"))
                .header("authorization", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(file.status(), StatusCode::OK);
    assert_eq!(file.headers()["content-type"], "text/plain; charset=utf-8");
    assert!(
        file.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("attachment.txt")
    );
    assert_eq!(
        file.into_body().collect().await.unwrap().to_bytes(),
        "fault details"
    );

    let invalid = app
        .clone()
        .oneshot(
            Request::patch(format!("/api/v1/device/faulty/report/{id}/status"))
                .header("authorization", token)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"status":"pending"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let approved = app
        .clone()
        .oneshot(
            Request::patch(format!("/api/v1/device/faulty/report/{id}/status"))
                .header("authorization", token)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"status":"approved"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approved.status(), StatusCode::OK);

    let deleted = app
        .oneshot(
            Request::delete(format!("/api/v1/device/faulty/report/{id}"))
                .header("authorization", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);
}

fn log_test_app() -> axum::Router {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    api::router(Arc::new(
        AppState::with_identity_provider(
            AppConfig::load(&cli).unwrap(),
            Metrics::new().unwrap(),
            Arc::new(AcceptingIdentityProvider),
        )
        .with_device_repository(Arc::new(RegistrationRepository {
            calls: AtomicUsize::new(0),
            callbacks: Mutex::new(Vec::new()),
        })),
    ))
}

fn execution_creation_test_app(with_catalog: bool) -> axum::Router {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state = AppState::with_identity_provider(
        AppConfig::load(&cli).unwrap(),
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    }));
    let state = if with_catalog {
        state.with_test_catalog(Arc::new(FakeTestCatalog))
    } else {
        state
    };
    api::router(Arc::new(state))
}

#[tokio::test]
async fn execution_creation_preserves_legacy_payload_and_auth_contract() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let state = AppState::with_identity_provider(
        AppConfig::load(&cli).unwrap(),
        Metrics::new().unwrap(),
        Arc::new(AcceptingIdentityProvider),
    )
    .with_device_repository(Arc::new(RegistrationRepository {
        calls: AtomicUsize::new(0),
        callbacks: Mutex::new(Vec::new()),
    }));
    let mut events = state.event_publisher.subscribe();
    let app = api::router(Arc::new(state));
    let payload = serde_json::json!({
        "name": "Smoke execution",
        "deviceFamily": "R-Car",
        "deviceType": "x5h",
        "buildId": "build-1",
        "testPlanId": 7,
        "testPlanName": "Smoke",
        "testCases": {
            "8": [{
                "testCaseId": 9,
                "suiteId": 8,
                "scriptFile": "run.sh",
                "suiteName": "Core",
                "title": "Boot",
                "planId": 7
            }]
        }
    });
    let unauthorized = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/test/execution")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/test/execution")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["testId"], "created-test-1");
    assert_eq!(body["status"], "not_executed");
    assert_eq!(body["executionPhase"], "PREPARE_ARTIFACTS");
    assert_eq!(body["testCases"][0]["testCaseId"], 9);
    assert_eq!(body["createdBy"], "system");
    let event = events.recv().await.unwrap();
    assert_eq!(event.event, "test_execution");
    assert_eq!(event.payload["testId"], body["testId"]);
    assert_eq!(event.payload["type"], "new");
    assert_eq!(event.payload["data"]["status"], body["status"]);
    assert_eq!(event.room.as_deref(), Some("user:system"));

    let invalid = app
        .oneshot(
            Request::post("/api/v1/device/test/execution")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "deviceFamily": "R-Car",
                        "deviceType": "x5h",
                        "buildId": "build-1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn execution_creation_expands_selection_through_test_catalog() {
    let response = execution_creation_test_app(true)
        .oneshot(
            Request::post("/api/v1/device/test/execution")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "deviceFamily": "R-Car",
                        "deviceType": "x5h",
                        "buildId": "build-1",
                        "selection": {
                            "mode": "PARTIAL",
                            "planId": "P7",
                            "planName": "Plan",
                            "suites": [{"suiteId": "S8", "selectAll": true}]
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["testPlanId"], 7);
    assert_eq!(body["testSuits"], serde_json::json!([8]));
    assert_eq!(body["testCases"][0]["testCaseId"], 9);
    assert_eq!(body["testCases"][0]["suiteName"], "Core");
    assert_eq!(body["selectionInput"]["mode"], "PARTIAL");
}

#[tokio::test]
async fn execution_update_preserves_validation_and_legacy_acknowledgement() {
    let app = execution_creation_test_app(true);
    let payload = serde_json::json!({
        "testID": "42",
        "result": "PASS",
        "comments": "completed"
    });
    let unauthorized = app
        .clone()
        .oneshot(
            Request::put("/api/v1/device/test/execution/9")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::put("/api/v1/device/test/execution/9")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "{}"
    );

    let invalid = app
        .oneshot(
            Request::put("/api/v1/device/test/execution/9")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"testID":"42","result":"UNKNOWN"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let unavailable = execution_creation_test_app(false)
        .oneshot(
            Request::put("/api/v1/device/test/execution/not-numeric")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"testID":"not-numeric","result":"FAIL","log":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unavailable.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn log_routes_require_authentication() {
    let response = log_test_app()
        .oneshot(
            Request::get("/api/v1/device/log/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_log_applies_defaults_and_validates_payload() {
    let app = log_test_app();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/device/log/")
                .header("authorization", "Bearer access")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "referenceId": "device-1",
                        "data": {"message": "ready"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["type"], "general");
    assert_eq!(body["level"], "info");
    assert_eq!(body["referenceId"], "device-1");

    for payload in [
        serde_json::json!({"referenceId": "", "data": {}}),
        serde_json::json!({"referenceId": "device-1", "data": []}),
        serde_json::json!({"type": "unknown", "referenceId": "device-1", "data": {}}),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/device/log")
                    .header("authorization", "Bearer access")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn list_and_search_logs_preserve_pagination_and_filters() {
    let app = log_test_app();
    let response = app
        .clone()
        .oneshot(
            Request::get(
                "/api/v1/device/log?type=device&referenceId=device-1&level=warn&page=0&pageSize=200&sort=referenceId&order=ASC",
            )
            .header("authorization", "Bearer access")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["page"], 1);
    assert_eq!(body["pageSize"], 100);
    assert_eq!(body["logs"][0]["type"], "device");
    assert_eq!(body["logs"][0]["referenceId"], "device-1");
    assert_eq!(body["logs"][0]["level"], "warn");

    let missing = app
        .clone()
        .oneshot(
            Request::get("/api/v1/device/log/search")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let response = app
        .oneshot(
            Request::get("/api/v1/device/log/search?query=panic&page=2&pageSize=5")
                .header("authorization", "Bearer access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["page"], 2);
    assert_eq!(body["pageSize"], 5);
    assert_eq!(body["logs"][0]["data"]["message"], "panic");
}
