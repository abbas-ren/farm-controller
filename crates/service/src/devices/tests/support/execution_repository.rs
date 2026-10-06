use super::*;
#[async_trait]
impl ExecutionLifecycleRepository for RegistrationRepository {
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
}

#[async_trait]
impl ExecutionReportRepository for RegistrationRepository {
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
}

#[async_trait]
impl ExecutionQueryRepository for RegistrationRepository {
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
}

#[async_trait]
impl BuildRepository for RegistrationRepository {
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
}
