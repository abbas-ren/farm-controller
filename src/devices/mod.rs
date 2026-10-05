use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{auth, error::ErrorResponse, events::ServerEvent, state::AppState};

pub(crate) mod alert_handlers;
mod alert_store;
pub(crate) mod analytics_handlers;
mod analytics_store;
pub(crate) mod artifact_handlers;
mod artifacts;
pub(crate) mod build_handlers;
mod build_ingestion;
pub(crate) mod build_store;
mod callback_store;
mod constants;
mod controller_store;
mod csv_export;
pub(crate) mod device_export_handlers;
mod device_export_store;
pub(crate) mod device_registration_handlers;
mod device_registration_store;
mod error;
pub(crate) mod faulty_report_handlers;
mod faulty_report_store;
pub(crate) mod flashing_handlers;
mod flashing_store;
mod heartbeat_store;
mod legacy_relay_store;
pub(crate) mod log_handlers;
mod registration_store;
mod relay_configuration_store;
pub(crate) mod relay_handlers;
mod relay_store;
mod repository;
pub(crate) mod repository_types;
mod routes;
pub(crate) mod test_catalog_handlers;
pub(crate) mod test_completion_handlers;
mod test_completion_store;
pub(crate) mod test_execution_handlers;
mod test_export;
pub(crate) mod test_export_handlers;
#[cfg(test)]
pub mod tests;
pub(crate) mod tus_handlers;
mod tus_store;
mod types;
mod upload_events;
mod validation;

pub(crate) use constants::EDGE_CONTROLLER_PORT;
pub(crate) use relay_handlers::relay_device_state;
pub(crate) use repository_types::RelayControllerAction;
pub(crate) use routes::router;

pub use types::{
    ActiveExecutionRecord, AdminAlertListQuery, AlertList, AllAlertListQuery, AvailableRelayDevice,
    AvailableRelayDevicesQuery, AvailableRelayDevicesResponse, BuildExecutionList, BuildFilters,
    BuildFlagRequest, BuildFlagResponse, BuildList, BuildListQuery, BuildUploadInitRequest,
    BuildUploadInitResponse, BuildUploadResponse, BuildsQuery, CallbackResponse, CallbackStatus,
    CompatibilityErrorResponse, ControllerEditRequest, ControllerHeartbeat, ControllerList,
    ControllerListQuery, ControllerRegistration, ControllerRegistrationResponse,
    CreateExecutionRequest, DefaultArtifactCopyQuery, DefaultArtifactCopyResponse, DeviceAction,
    DeviceActionRequest, DeviceController, DeviceCsvQuery, DeviceDataResponse,
    DeviceFlashingResponse, DeviceHeartbeatResponse, DeviceList, DeviceListQuery,
    DeviceRegistration, DeviceRegistrationResponse, DeviceStateAnalytics,
    DeviceStateDetailedAnalytics, DeviceStateDetailedItem, DeviceTopologyResponse,
    DeviceTypeFolder, DeviceTypeFolderRequest, EmptyObjectResponse, ExecutionCaseInput,
    ExecutionCaseRecord, ExecutionList, ExecutionListQuery, ExecutionReportRecord,
    ExecutionSelection, ExecutionSuiteSelection, FaultyReportCreateRequest, FaultyReportDetail,
    FaultyReportRecord, FaultyReportStatus, FaultyReportStatusRequest, FaultyReportUser,
    HeartbeatTimeoutRequest, LegacyRelayChannelListQuery, LegacyRelayConfiguration,
    LegacyRelayConfigurationResponse, LegacyRelayConflictQuery, LegacyRelayListQuery,
    LegacyRelayRemapRequest, LogCreateRequest, LogEntry, LogList, LogListQuery, MappingCallback,
    MessageResponse, PaginationResponse, Relay, RelayChannelConfiguration, RelayChannelRecord,
    RelayChannelRegistration, RelayConfigurationAccepted, RelayConfigurationConfirmation,
    RelayConfigurationRequest, RelayConfirmationResponse, RelayConflictResponse,
    RelayConflictState, RelayIdentityUpdateRequest, RelayIdentityUpdateResponse, RelayRecord,
    RelayRegistration, SuccessResponse, TestCompletionQuery, TestCompletionResponse,
    UpdateExecutionRequest, UserDeviceListQuery, VoltageLevel,
};

use constants::{DEVICE_CALLBACK_TIMEOUT, MAX_PAGE_SIZE};
use error::DeviceRepositoryError;
pub(crate) use repository::DeviceRepository;
use repository_types::{
    BuildDeleteTarget, BuildUploadFinalization, BuildUploadResult, ControllerDeleteTarget,
    ControllerHeartbeatResult, DeviceActionTarget, DeviceDeleteTarget, DeviceFlashingResult,
    DeviceHeartbeatResult, DevicePowerTarget, DeviceRebootTarget, ExecutionCreation,
    FaultyReportCreate, FaultyReportCreation, FlashConfirmationResult, RegistrationResult,
    RelayConfigurationResult, RelayIdentityTarget, RelayStateTarget, TestCancellationResult,
    TestCompletionTarget, TestExportRow,
};
use types::{DeviceTypesQuery, FlashConfirmationQuery, TtyEntry};

pub(crate) struct PostgresDeviceRepository {
    pool: PgPool,
}

impl PostgresDeviceRepository {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DeviceRepository for PostgresDeviceRepository {
    async fn create_test_execution(
        &self,
        execution: &ExecutionCreation,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let release = sqlx::query_as::<_, (String, bool)>(
            r#"SELECT version, "isFaulty" FROM releases WHERE id::text = $1 FOR SHARE"#,
        )
        .bind(&execution.build_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::NotFound(format!("Release {} not found", execution.build_id))
        })?;
        if release.1 {
            return Err(DeviceRepositoryError::Validation(format!(
                "Cannot create test execution: Release {} is marked as faulty",
                execution.build_id
            )));
        }
        let test_id = Uuid::new_v4().simple().to_string();
        let name = execution
            .name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "{} | {}:{} | {}",
                    execution.plan_name,
                    execution.device_type,
                    release.0,
                    chrono::Utc::now().format("%b %-d, %Y %-I:%M %p")
                )
            });
        let test_suites = serde_json::to_value(&execution.test_suites)
            .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
        let selection = execution
            .selection
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
        sqlx::query(
            r#"INSERT INTO test_executions
               ("testId", "deviceFamily", "deviceType", "deviceId", "buildId", "buildVersion",
                "testPlanId", "testSuits", "selectionInput", name, status, "createdBy",
                "testPlanName", "isAllSelected", "executionPhase", logs, "createdAt", "updatedAt")
               VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8, $9, 'not_executed', $10,
                       $11, $12, 'PREPARE_ARTIFACTS', '[]'::jsonb, now(), now())"#,
        )
        .bind(&test_id)
        .bind(&execution.device_family)
        .bind(&execution.device_type)
        .bind(&execution.build_id)
        .bind(&release.0)
        .bind(execution.plan_id)
        .bind(test_suites)
        .bind(selection)
        .bind(name)
        .bind(&execution.user_id)
        .bind(&execution.plan_name)
        .bind(execution.is_all_selected)
        .execute(&mut *transaction)
        .await?;
        for case in &execution.cases {
            let result = case
                .result
                .as_deref()
                .filter(|value| matches!(*value, "PASS" | "FAIL"));
            sqlx::query(
                r#"INSERT INTO testcase
                   ("testCaseId", title, "executionId", result, "suiteId", "scriptFile", "planId",
                    "priorityId", "order", "suiteName", labels, "preCondition", "createdAt", "updatedAt")
                   VALUES ($1, $2, $3, $4::"enum_testcase_result", $5, $6, $7, $8, $9, $10, $11, $12, now(), now())"#,
            )
            .bind(case.test_case_id)
            .bind(&case.title)
            .bind(&test_id)
            .bind(result)
            .bind(case.suite_id)
            .bind(&case.script_file)
            .bind(case.plan_id)
            .bind(case.priority_id)
            .bind(case.order)
            .bind(&case.suite_name)
            .bind(&case.labels)
            .bind(&case.pre_condition)
            .execute(&mut *transaction)
            .await?;
        }
        let created = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(te) || jsonb_build_object(
                 'Device', NULL,
                 'testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc."suiteId", tc.id)
                                        FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb),
                 'logs', '[]'::jsonb)
               FROM test_executions te WHERE te."testId" = $1"#,
        )
        .bind(&test_id)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(created)
    }

    async fn test_execution_build_version(
        &self,
        test_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT "buildVersion" FROM test_executions WHERE "testId" = $1"#,
        )
        .bind(test_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    async fn create_log(
        &self,
        request: &LogCreateRequest,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"INSERT INTO log_entries (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
               VALUES ($1::"enum_log_entries_type", $2, $3, $4::"enum_log_entries_level", COALESCE($5, now()), now(), now())
               RETURNING to_jsonb(log_entries)"#,
        )
        .bind(&request.log_type)
        .bind(&request.reference_id)
        .bind(&request.data)
        .bind(&request.level)
        .bind(request.timestamp)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn list_logs(
        &self,
        query: &LogListQuery,
        search: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let page_size = query
            .page_size
            .unwrap_or(20)
            .clamp(1, i64::from(MAX_PAGE_SIZE));
        let offset = (page - 1) * page_size;
        let sort = match query.sort.as_deref() {
            Some("id") => "id",
            Some("type") => "type",
            Some("referenceId") => "referenceId",
            Some("level") => "level",
            Some("createdAt") => "createdAt",
            Some("updatedAt") => "updatedAt",
            _ => "timestamp",
        };
        let order = if query.order.as_deref() == Some("ASC") {
            "ASC"
        } else {
            "DESC"
        };
        let total = sqlx::query_scalar::<_, i64>(
            r#"SELECT count(*) FROM log_entries
                             WHERE ($1::text IS NULL OR type::text = $1)
                                 AND ($2::text IS NULL OR "referenceId" = $2)
                                 AND ($3::text IS NULL OR level::text = $3)
                                 AND ($4::text IS NULL OR data::text ILIKE '%' || $4 || '%')"#,
        )
        .bind(query.log_type.as_deref())
        .bind(query.reference_id.as_deref())
        .bind(query.level.as_deref())
        .bind(search)
        .fetch_one(&self.pool)
        .await?;
        let logs = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(log_entries) FROM log_entries
                WHERE ($1::text IS NULL OR type::text = $1)
                  AND ($2::text IS NULL OR "referenceId" = $2)
                  AND ($3::text IS NULL OR level::text = $3)
                  AND ($4::text IS NULL OR data::text ILIKE '%' || $4 || '%')
                ORDER BY
                  CASE WHEN $5 = 'id' AND $6 = 'ASC' THEN id END ASC,
                  CASE WHEN $5 = 'id' AND $6 = 'DESC' THEN id END DESC,
                  CASE WHEN $5 = 'type' AND $6 = 'ASC' THEN type::text END ASC,
                  CASE WHEN $5 = 'type' AND $6 = 'DESC' THEN type::text END DESC,
                  CASE WHEN $5 = 'referenceId' AND $6 = 'ASC' THEN "referenceId" END ASC,
                  CASE WHEN $5 = 'referenceId' AND $6 = 'DESC' THEN "referenceId" END DESC,
                  CASE WHEN $5 = 'level' AND $6 = 'ASC' THEN level::text END ASC,
                  CASE WHEN $5 = 'level' AND $6 = 'DESC' THEN level::text END DESC,
                  CASE WHEN $5 = 'createdAt' AND $6 = 'ASC' THEN "createdAt" END ASC,
                  CASE WHEN $5 = 'createdAt' AND $6 = 'DESC' THEN "createdAt" END DESC,
                  CASE WHEN $5 = 'updatedAt' AND $6 = 'ASC' THEN "updatedAt" END ASC,
                  CASE WHEN $5 = 'updatedAt' AND $6 = 'DESC' THEN "updatedAt" END DESC,
                  CASE WHEN $5 = 'timestamp' AND $6 = 'ASC' THEN timestamp END ASC,
                  CASE WHEN $5 = 'timestamp' AND $6 = 'DESC' THEN timestamp END DESC
                LIMIT $7 OFFSET $8"#,
        )
        .bind(query.log_type.as_deref())
        .bind(query.reference_id.as_deref())
        .bind(query.level.as_deref())
        .bind(search)
        .bind(sort)
        .bind(order)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(serde_json::json!({
            "logs": logs,
            "total": total,
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
        let total = sqlx::query_scalar::<_, i64>(
            r#"SELECT count(*) FROM test_executions WHERE "buildId" = $1"#,
        )
        .bind(build_id)
        .fetch_one(&self.pool)
        .await?;
        let executions = sqlx::query_scalar::<_, serde_json::Value>(r#"SELECT to_jsonb(te) || jsonb_build_object('testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id) FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb)) FROM test_executions te WHERE te."buildId" = $1 ORDER BY te."createdAt" DESC LIMIT $2 OFFSET $3"#).bind(build_id).bind(limit).bind(offset).fetch_all(&self.pool).await?;
        Ok(
            serde_json::json!({"limit": limit, "offset": offset, "total": total, "executions": executions}),
        )
    }

    async fn test_results(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(r#"SELECT jsonb_build_object('status', te.status, 'testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id) FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb)) FROM test_executions te WHERE te."testId" = $1"#).bind(test_id).fetch_optional(&self.pool).await?)
    }

    async fn request_test_cancellation(
        &self,
        test_id: &str,
        user_id: &str,
        nfs_host_path: &str,
    ) -> Result<Option<TestCancellationResult>, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query_as::<
            _,
            (
                bool,
                String,
                String,
                Option<String>,
                Option<String>,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(
            r#"SELECT COALESCE(execution."cancelRequested", false), execution.status::text,
                      execution."buildId"::text, execution."executionPhase"::text,
                      execution."deviceId", execution."deviceType", execution."buildVersion",
                      execution."testCycleId", device."ipAddress", device."nfsPath",
                      execution."createdBy"
               FROM test_executions execution
               LEFT JOIN devices device ON device."deviceId" = execution."deviceId"
               WHERE execution."testId" = $1 FOR UPDATE OF execution"#,
        )
        .bind(test_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((
            requested,
            status,
            build_id,
            phase,
            device_id,
            device_type,
            build_version,
            test_cycle_id,
            device_ip,
            device_nfs_path,
            created_by,
        )) = row
        else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let changed =
            !requested && !matches!(status.as_str(), "cancelled" | "completed" | "failed");
        let phase = phase.unwrap_or_else(|| "PREPARE_ARTIFACTS".to_owned());
        let deferred = matches!(
            phase.as_str(),
            "WAIT_FOR_IPL_CONFIRMATION" | "WAIT_FOR_DEVICE_BOOT"
        );
        let already_finalizing = matches!(
            phase.as_str(),
            "SEND_FALLBACK_FLASH_REQUEST"
                | "WAIT_FOR_FALLBACK_COMPLETION"
                | "COMPLETE_SUCCESS"
                | "COMPLETE_FAILED"
                | "COMPLETE_CANCELLED"
        );
        let terminalized = changed && !deferred && !already_finalizing;
        if changed {
            sqlx::query(r#"UPDATE test_executions SET "cancelRequested" = true, "cancelRequestedAt" = now(), "cancelRequestedBy" = $2, "updatedAt" = now() WHERE "testId" = $1"#).bind(test_id).bind(user_id).execute(&mut *transaction).await?;
            sqlx::query(
                r#"INSERT INTO log_entries
                      (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                   VALUES ('test', $1, jsonb_build_object('message', $2::text), 'info',
                           now(), now(), now())"#,
            )
            .bind(test_id)
            .bind(format!("Cancel requested during phase: {phase}"))
            .execute(&mut *transaction)
            .await?;
        }
        if terminalized {
            sqlx::query(
                r#"UPDATE test_executions
                   SET status = 'cancelled', "cancelHandled" = true,
                       "executionPhase" = 'COMPLETE_CANCELLED',
                       "startedAt" = COALESCE("startedAt", now()), "endedAt" = now(),
                       "updatedAt" = now()
                   WHERE "testId" = $1
                     AND status::text NOT IN ('cancelled', 'completed', 'failed')"#,
            )
            .bind(test_id)
            .execute(&mut *transaction)
            .await?;
            if phase == "START_TEST_EXECUTION"
                && device_nfs_path
                    .as_deref()
                    .is_some_and(|path| path.contains(test_id))
                && let Some(device_id) = device_id.as_deref()
            {
                let fallback_path = std::path::Path::new(nfs_host_path)
                    .join(&device_type)
                    .join(build_version.as_deref().unwrap_or(&build_id));
                sqlx::query(
                    r#"INSERT INTO fallback_updates
                          ("deviceId", "releaseId", "testId", "fallbackPath", status,
                           "createdAt", "updatedAt")
                       SELECT $1, $2, $3, $4, 'pending', now(), now()
                       WHERE NOT EXISTS (
                           SELECT 1 FROM fallback_updates
                           WHERE "testId" = $3
                             AND status::text IN ('pending', 'flashing', 'completed')
                       )"#,
                )
                .bind(device_id)
                .bind(&build_id)
                .bind(test_id)
                .bind(fallback_path.to_string_lossy().as_ref())
                .execute(&mut *transaction)
                .await?;
            }
            sqlx::query(
                r#"UPDATE device_action_queue SET status = 'cancelled', "updatedAt" = now()
                   WHERE "testId" = $1
                     AND status::text NOT IN ('completed', 'cancelled')"#,
            )
            .bind(test_id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                r#"INSERT INTO log_entries
                      (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                   VALUES ('test', $1, jsonb_build_object('message', $2::text), 'info',
                           now(), now(), now())"#,
            )
            .bind(test_id)
            .bind("This test has been cancelled successfully")
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(Some(TestCancellationResult {
            build_id,
            created_by: created_by.unwrap_or_else(|| user_id.to_owned()),
            cleanup_device_type: terminalized
                .then_some(phase.as_str())
                .filter(|phase| matches!(*phase, "PREPARE_ARTIFACTS" | "PREPARE_TEST_SCRIPTS"))
                .map(|_| device_type),
            device_ip: (terminalized && phase == "START_TEST_EXECUTION")
                .then_some(device_ip)
                .flatten(),
            phase,
            changed,
            terminalized,
            test_cycle_id,
        }))
    }

    async fn report_generation_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, (String, String)>(r#"SELECT "deviceType", COALESCE("buildVersion", '') FROM test_executions WHERE "testId" = $1 AND status::text = 'completed'"#).bind(test_id).fetch_optional(&self.pool).await?)
    }

    async fn create_execution_report_record(
        &self,
        report_id: Uuid,
        test_id: &str,
        device_type: &str,
        build_version: &str,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let active = sqlx::query_scalar::<_, String>(r#"SELECT status::text FROM execution_reports WHERE "testExecutionId" = $1 AND status::text IN ('generating', 'uploading') LIMIT 1 FOR UPDATE"#).bind(test_id).fetch_optional(&mut *transaction).await?;
        if let Some(status) = active {
            return Err(DeviceRepositoryError::Conflict(format!(
                "A report for this execution is already {status}"
            )));
        }
        sqlx::query(r#"DELETE FROM execution_reports WHERE "testExecutionId" = $1"#)
            .bind(test_id)
            .execute(&mut *transaction)
            .await?;
        let report = sqlx::query_scalar::<_, serde_json::Value>(r#"INSERT INTO execution_reports (id, "testExecutionId", status, "deviceType", "buildVersion", "createdBy", "createdAt", "updatedAt") VALUES ($1, $2, 'generating', $3, $4, $5, now(), now()) RETURNING to_jsonb(execution_reports)"#)
            .bind(report_id).bind(test_id).bind(device_type).bind(build_version).bind(user_id).fetch_one(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(report)
    }

    async fn update_execution_report_status(
        &self,
        report_id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DeviceRepositoryError> {
        sqlx::query(r#"UPDATE execution_reports SET status = $2::"enum_execution_reports_status", "uploadError" = $3, "updatedAt" = now() WHERE id = $1"#).bind(report_id).bind(status).bind(error).execute(&self.pool).await?;
        Ok(())
    }

    async fn execution_report(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(report) FROM execution_reports report
               WHERE "testExecutionId" = $1 ORDER BY "createdAt" DESC LIMIT 1"#,
        )
        .bind(test_id)
        .fetch_optional(&self.pool)
        .await?)
    }
    async fn execution_report_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, (String, String)>(
            r#"SELECT "deviceType", COALESCE("buildVersion", '') FROM test_executions WHERE "testId" = $1"#,
        ).bind(test_id).fetch_optional(&self.pool).await?)
    }

    async fn test_case_log_path(
        &self,
        case_id: i64,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT "outputFilePath" FROM testcase WHERE id = $1"#,
        )
        .bind(case_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    async fn execution_cases(
        &self,
        user_id: &str,
        test_id: &str,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
        let owned = sqlx::query_scalar::<_, bool>(r#"SELECT EXISTS(SELECT 1 FROM test_executions WHERE "testId" = $1 AND "createdBy" = $2)"#).bind(test_id).bind(user_id).fetch_one(&self.pool).await?;
        if !owned {
            return Ok(None);
        }
        Ok(Some(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT jsonb_build_object('testCaseId', "testCaseId", 'suiteId', "suiteId", 'result', result,
                 'comment', comment, 'jiraDefect', "jiraDefect", 'title', title, 'suiteName', "suiteName",
                 'scriptFile', "scriptFile", 'id', id, 'updatedAt', "updatedAt",
                 'outputFilePath', "outputFilePath", 'dmesgFilePath', "dmesgFilePath")
               FROM testcase WHERE "executionId" = $1 ORDER BY id"#,
        ).bind(test_id).fetch_all(&self.pool).await?))
    }

    async fn test_case(
        &self,
        case_id: i64,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(tc) FROM testcase tc WHERE id = $1"#,
        )
        .bind(case_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn list_test_executions(
        &self,
        user_id: &str,
        query: &ExecutionListQuery,
    ) -> Result<ExecutionList, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(10).max(1);
        let search = query.search.as_deref().unwrap_or_default();
        let sort_by = query
            .sort_by
            .as_deref()
            .filter(|value| {
                matches!(
                    *value,
                    "createdAt" | "status" | "testPlanName" | "buildVersion" | "deviceType"
                )
            })
            .unwrap_or("createdAt");
        let descending = query.desc.as_deref().unwrap_or("true") == "true";
        let total = sqlx::query_scalar::<_, i64>(
            r#"SELECT count(*) FROM test_executions te LEFT JOIN devices d ON d."deviceId" = te."deviceId"
               WHERE te."createdBy" = $1 AND ($2 = '' OR d."deviceType" ILIKE '%' || $2 || '%')"#,
        ).bind(user_id).bind(search).fetch_one(&self.pool).await?;
        let data = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT jsonb_build_object(
                 'testId', te."testId", 'durationSeconds', GREATEST(0, floor(extract(epoch FROM (COALESCE(te."endedAt", now()) - COALESCE(te."startedAt", now())))))::bigint,
                 'total', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId"),
                 'passed', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId" AND tc.result::text = 'PASS'),
                 'failed', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId" AND tc.result::text = 'FAIL'),
                 'buildId', te."buildId", 'buildVersion', te."buildVersion", 'deviceName', COALESCE(d."deviceType", ''),
                 'deviceId', COALESCE(te."deviceId", ''), 'status', te.status, 'testCases', '{}'::jsonb,
                 'testPlanName', te."testPlanName", 'logs', '[]'::jsonb, 'testCycleId', COALESCE(te."testCycleId", ''),
                 'createdAt', te."createdAt", 'rtosLogPath', te."rtosLogPath")
               FROM test_executions te LEFT JOIN devices d ON d."deviceId" = te."deviceId"
               WHERE te."createdBy" = $1 AND ($2 = '' OR d."deviceType" ILIKE '%' || $2 || '%')
               ORDER BY CASE WHEN $3 = 'createdAt' AND $4 THEN te."createdAt" END DESC,
                 CASE WHEN $3 = 'createdAt' AND NOT $4 THEN te."createdAt" END ASC,
                 CASE WHEN $3 = 'status' AND $4 THEN te.status::text END DESC,
                 CASE WHEN $3 = 'status' AND NOT $4 THEN te.status::text END ASC,
                 CASE WHEN $3 = 'testPlanName' AND $4 THEN te."testPlanName" END DESC,
                 CASE WHEN $3 = 'testPlanName' AND NOT $4 THEN te."testPlanName" END ASC,
                 CASE WHEN $3 = 'buildVersion' AND $4 THEN te."buildVersion" END DESC,
                 CASE WHEN $3 = 'buildVersion' AND NOT $4 THEN te."buildVersion" END ASC,
                 CASE WHEN $3 = 'deviceType' AND $4 THEN d."deviceType" END DESC,
                 CASE WHEN $3 = 'deviceType' AND NOT $4 THEN d."deviceType" END ASC
               LIMIT $5 OFFSET $6"#,
        ).bind(user_id).bind(search).bind(sort_by).bind(descending).bind(limit).bind((page - 1) * limit).fetch_all(&self.pool).await?;
        Ok(ExecutionList {
            data,
            total: total as u64,
            current_page: page as u64,
            total_pages: ((total + limit - 1) / limit).max(0) as u64,
        })
    }

    async fn test_execution_by_device(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(te) FROM test_executions te
               WHERE te."deviceId" = $1 AND te."createdBy" = $2
               ORDER BY te."updatedAt" DESC LIMIT 1"#,
        )
        .bind(device_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn in_progress_test_executions(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT jsonb_build_object('deviceId', "deviceId", 'status', status, 'testId', "testId")
               FROM test_executions WHERE "createdBy" = $1
                 AND status::text IN ('in_progress', 'not_executed', 'queued')"#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn single_test_execution(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(te) || jsonb_build_object('testCases', COALESCE((
                   SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id) FROM testcase tc
                   WHERE tc."executionId" = te."testId"), '[]'::jsonb))
               FROM test_executions te WHERE te."testId" = $1"#,
        )
        .bind(test_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn execution_case_ids(
        &self,
        test_id: &str,
    ) -> Result<Option<Vec<String>>, DeviceRepositoryError> {
        let exists = sqlx::query_scalar::<_, bool>(
            r#"SELECT EXISTS(SELECT 1 FROM test_executions
               WHERE "testId" = $1 AND status::text NOT IN ('cancelled', 'failed'))"#,
        )
        .bind(test_id)
        .fetch_one(&self.pool)
        .await?;
        if !exists {
            return Ok(None);
        }
        Ok(Some(
            sqlx::query_scalar::<_, String>(
                r#"SELECT "testCaseId"::text FROM testcase
               WHERE "executionId" = $1 ORDER BY "suiteId", id"#,
            )
            .bind(test_id)
            .fetch_all(&self.pool)
            .await?,
        ))
    }

    async fn test_execution(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(te) || jsonb_build_object('Device', CASE WHEN d."deviceId" IS NULL THEN NULL ELSE to_jsonb(d) END)
               FROM test_executions te LEFT JOIN devices d ON d."deviceId" = te."deviceId"
               WHERE te."createdBy" = $1
                 AND (($2 = 'latest' AND te.status::text IN ('not_executed', 'queued', 'in_progress')) OR te."testId" = $2)
               ORDER BY CASE WHEN $2 = 'latest' THEN te."createdAt" END DESC LIMIT 1"#,
        )
        .bind(user_id)
        .bind(execution_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn test_execution_summary(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let Some(mut summary) = sqlx::query_scalar::<_, serde_json::Value>(
                        r#"SELECT jsonb_build_object(
                                 'testId', te."testId",
                                 'durationSeconds', GREATEST(0, floor(extract(epoch FROM (COALESCE(te."endedAt", now()) - COALESCE(te."startedAt", now())))))::bigint,
                                 'total', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId"),
                                 'passed', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId" AND tc.result::text = 'PASS'),
                                 'failed', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId" AND tc.result::text = 'FAIL'),
                                 'buildId', te."buildId", 'buildVersion', te."buildVersion",
                                 'deviceName', COALESCE(d."deviceType", ''), 'deviceType', COALESCE(d."deviceType", ''),
                                 'status', te.status,
                                 'testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id) FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb),
                                 'testPlanName', te."testPlanName", 'testCycleId', te."testCycleId",
                                 'createdAt', te."createdAt", 'rtosLogPath', te."rtosLogPath")
                             FROM test_executions te
                             LEFT JOIN devices d ON d."deviceId" = te."deviceId"
                             WHERE te."testId" = $1"#,
                )
                .bind(execution_id)
                .fetch_optional(&self.pool)
                .await?
                else {
                        return Ok(None);
                };
        let analytics = sqlx::query_scalar::<_, serde_json::Value>(
                        r#"WITH eligible AS (
                                 SELECT te."createdAt", tc.result::text AS result
                                 FROM test_executions te
                                 JOIN testcase tc ON tc."executionId" = te."testId"
                                 WHERE te."createdBy" = $1
                                     AND te.status::text NOT IN ('cancelled', 'queued', 'not_executed', 'failed')
                             )
                             SELECT jsonb_build_object(
                                 'allTime', jsonb_build_object(
                                     'total', count(*), 'passed', count(*) FILTER (WHERE result = 'PASS'),
                                     'failed', count(*) FILTER (WHERE result = 'FAIL'),
                                     'inProgress', count(*) FILTER (WHERE result IS NULL OR result NOT IN ('PASS', 'FAIL'))),
                                 'currentWeek', jsonb_build_object(
                                     'week', to_char(date_trunc('week', now()), 'DD Mon') || ' - ' || to_char(date_trunc('week', now()) + interval '6 days', 'DD Mon'),
                                     'total', count(*) FILTER (WHERE "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week'),
                                     'passed', count(*) FILTER (WHERE result = 'PASS' AND "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week'),
                                     'failed', count(*) FILTER (WHERE result = 'FAIL' AND "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week'),
                                     'inProgress', count(*) FILTER (WHERE (result IS NULL OR result NOT IN ('PASS', 'FAIL')) AND "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week')),
                                 'currentMonth', jsonb_build_object(
                                     'month', to_char(date_trunc('month', now()), 'Mon YYYY'),
                                     'total', count(*) FILTER (WHERE "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month'),
                                     'passed', count(*) FILTER (WHERE result = 'PASS' AND "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month'),
                                     'failed', count(*) FILTER (WHERE result = 'FAIL' AND "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month'),
                                     'inProgress', count(*) FILTER (WHERE (result IS NULL OR result NOT IN ('PASS', 'FAIL')) AND "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month')))
                             FROM eligible"#,
                )
                .bind(user_id)
                .fetch_one(&self.pool)
                .await?;
        summary["analytics"] = analytics;
        Ok(Some(summary))
    }

    async fn test_export_rows(
        &self,
        user_id: &str,
    ) -> Result<Vec<TestExportRow>, DeviceRepositoryError> {
        test_export::rows(&self.pool, user_id).await
    }

    async fn flag_build(
        &self,
        build_id: &str,
        request: &BuildFlagRequest,
    ) -> Result<Option<repository_types::BuildFlagResult>, DeviceRepositoryError> {
        build_store::flag_build(&self.pool, build_id, request.is_faulty).await
    }

    async fn delete_build(
        &self,
        build_id: &str,
    ) -> Result<Option<BuildDeleteTarget>, DeviceRepositoryError> {
        build_store::delete_build(&self.pool, build_id).await
    }

    async fn finalize_build_upload(
        &self,
        request: &BuildUploadFinalization,
    ) -> Result<BuildUploadResult, DeviceRepositoryError> {
        build_store::finalize_upload(&self.pool, request).await
    }

    async fn init_build_upload(
        &self,
        file_count: i32,
        user_id: &str,
    ) -> Result<Uuid, DeviceRepositoryError> {
        build_store::init_upload(&self.pool, file_count, user_id).await
    }

    async fn mark_build_upload_started(
        &self,
        upload_id: &str,
        user_id: &str,
    ) -> Result<(), DeviceRepositoryError> {
        build_store::mark_upload_started(&self.pool, upload_id, user_id).await
    }

    async fn mark_build_upload_failed(&self, upload_id: &str) -> Result<(), DeviceRepositoryError> {
        build_store::mark_upload_failed(&self.pool, upload_id).await
    }

    async fn build_filters(&self) -> Result<BuildFilters, DeviceRepositoryError> {
        build_store::build_filters(&self.pool).await
    }

    async fn build_by_id(
        &self,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        build_store::build_by_id(&self.pool, build_id).await
    }

    async fn list_builds(
        &self,
        query: &BuildListQuery,
    ) -> Result<BuildList, DeviceRepositoryError> {
        build_store::list_builds(&self.pool, query).await
    }

    async fn test_completion_target(
        &self,
        test_id: &str,
        device_id: &str,
    ) -> Result<Option<TestCompletionTarget>, DeviceRepositoryError> {
        test_completion_store::test_completion_target(&self.pool, test_id, device_id).await
    }

    async fn store_rtos_log_path(
        &self,
        test_id: &str,
        path: &str,
    ) -> Result<(), DeviceRepositoryError> {
        test_completion_store::store_rtos_log_path(&self.pool, test_id, path).await
    }

    async fn mark_device_flashing(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceFlashingResult>, DeviceRepositoryError> {
        flashing_store::mark_device_flashing(&self.pool, device_id).await
    }

    async fn export_devices(
        &self,
        query: &DeviceCsvQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        device_export_store::export_devices(&self.pool, query).await
    }

    async fn register_device(
        &self,
        registration: &DeviceRegistration,
    ) -> Result<repository_types::DeviceRegistrationResult, DeviceRepositoryError> {
        device_registration_store::register_device(&self.pool, registration).await
    }

    async fn register_controller(
        &self,
        registration: &ControllerRegistration,
    ) -> Result<RegistrationResult, DeviceRepositoryError> {
        registration_store::register_controller(&self.pool, registration).await
    }

    async fn save_gen5_mapping(
        &self,
        caller_ip: &str,
        mac: &str,
        status: CallbackStatus,
        tty_entry: Option<&TtyEntry>,
    ) -> Result<(), DeviceRepositoryError> {
        callback_store::save_gen5_mapping(&self.pool, caller_ip, mac, status, tty_entry).await
    }

    async fn confirm_flash(
        &self,
        device_type: &str,
        status: CallbackStatus,
    ) -> Result<FlashConfirmationResult, DeviceRepositoryError> {
        callback_store::confirm_flash(&self.pool, device_type, status).await
    }

    async fn device_families(&self) -> Result<Vec<String>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, String>(
            r#"
            SELECT DISTINCT "deviceFamily"
            FROM devices
            WHERE "deletedAt" IS NULL AND "deviceFamily" IS NOT NULL
            ORDER BY "deviceFamily"
            "#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn device_types(
        &self,
        device_family: Option<&str>,
    ) -> Result<Vec<String>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, String>(
            r#"
            SELECT DISTINCT "deviceType"
            FROM devices
            WHERE "deletedAt" IS NULL AND "deviceType" IS NOT NULL
              AND ($1::text IS NULL OR $1 = 'ALL' OR "deviceFamily" = $1)
            ORDER BY "deviceType"
            "#,
        )
        .bind(device_family)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn device_by_id(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT to_jsonb(device_row) || jsonb_build_object(
                'interfaces', COALESCE((
                    SELECT jsonb_agg(to_jsonb(interface_row) ORDER BY interface_row.id)
                    FROM device_interfaces interface_row
                    WHERE interface_row."deviceId" = device_row."deviceId"
                ), '[]'::jsonb),
                'testExecutions', COALESCE((
                    SELECT jsonb_agg(to_jsonb(execution_row))
                    FROM (
                        SELECT * FROM test_executions
                        WHERE "deviceId" = device_row."deviceId"
                        ORDER BY "updatedAt" DESC LIMIT 1
                    ) execution_row
                ), '[]'::jsonb)
            )
            FROM devices device_row
            WHERE device_row."deviceId" = $1 AND device_row."deletedAt" IS NULL
            "#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn latest_heartbeat(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let heartbeat = sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT jsonb_build_object(
                'timestamp', timestamp,
                'data', data,
                'timeout', timeout
            )
            FROM heartbeats
            WHERE "deviceId" = $1
              AND NOT (data @> '{"status":"disconnected"}'::jsonb)
            ORDER BY timestamp DESC LIMIT 1
            "#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?;
        if heartbeat.is_some() {
            return Ok(heartbeat);
        }
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT jsonb_build_object(
                'timestamp', timestamp,
                'data', data,
                'timeout', timeout
            )
            FROM device_controller_heartbeats
            WHERE "controllerId" = $1
              AND NOT (data @> '{"status":"disconnected"}'::jsonb)
            ORDER BY timestamp DESC LIMIT 1
            "#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn topology(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT jsonb_build_object(
                'deviceId', device."deviceId",
                'deviceName', device."deviceName",
                'deviceType', device."deviceType",
                'deviceFamily', device."deviceFamily",
                'macAddress', device."macAddress",
                'ipAddress', device."ipAddress",
                'state', device.state,
                'status', device.status,
                'controllerId', COALESCE((
                    SELECT relay."deviceControllerId"
                    FROM relay_channels channel
                    JOIN relays relay ON relay.id = channel."relayId"
                    WHERE channel."deviceId" = device."deviceId"
                      AND channel."deletedAt" IS NULL
                      AND relay."deletedAt" IS NULL
                    ORDER BY relay."updatedAt" DESC LIMIT 1
                ), device."controllerId"),
                'heartbeatTimer', device."heartbeatTimer"
            )
            FROM devices device
            WHERE device.status = 'approved' AND device."deletedAt" IS NULL
            ORDER BY device."deviceName" NULLS LAST, device."deviceId"
            "#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn list_devices(
        &self,
        query: &DeviceListQuery,
        is_admin: bool,
    ) -> Result<DeviceList, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(16).clamp(1, MAX_PAGE_SIZE);
        let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
        let search = query.search.as_deref().unwrap_or("").trim();
        let filter_by = match query.filter_by.as_deref().unwrap_or("") {
            "deviceType" | "deviceFamily" | "status" | "state" => {
                query.filter_by.as_deref().unwrap_or("")
            }
            _ => "",
        };
        let filter = query.filter.as_deref().unwrap_or("").trim();
        let sort_by = match query.sort_by.as_deref() {
            Some("updatedAt" | "deviceName" | "status") => {
                query.sort_by.as_deref().unwrap_or("createdAt")
            }
            _ => "createdAt",
        };
        let descending = query.desc.unwrap_or(true);
        let total_devices: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM devices d WHERE
            d."deletedAt" IS NULL
            AND ($1 OR d.status::text = 'approved')
            AND ($2 = '' OR d."deviceName" ILIKE '%' || $2 || '%'
                OR d."deviceId" ILIKE '%' || $2 || '%'
                OR d."deviceType" ILIKE '%' || $2 || '%'
                OR d."macAddress" ILIKE '%' || $2 || '%')
            AND ($3 = '' OR lower($4) = 'all' OR CASE $3
                WHEN 'deviceType' THEN d."deviceType" ILIKE '%' || $4 || '%'
                WHEN 'deviceFamily' THEN d."deviceFamily" ILIKE '%' || $4 || '%'
                WHEN 'status' THEN d.status::text ILIKE '%' || $4 || '%'
                WHEN 'state' THEN d.state::text ILIKE '%' || $4 || '%'
                ELSE TRUE END)
            "#,
        )
        .bind(is_admin)
        .bind(search)
        .bind(filter_by)
        .bind(filter)
        .fetch_one(&self.pool)
        .await?;
        let rows = sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT to_jsonb(d) || jsonb_build_object(
                'interfaces', COALESCE((
                    SELECT jsonb_agg(to_jsonb(di) ORDER BY di.id)
                    FROM device_interfaces di WHERE di."deviceId" = d."deviceId"
                ), '[]'::jsonb),
                'timeout', COALESCE((
                    SELECT h.timeout FROM heartbeats h
                    WHERE h."deviceId" = d."deviceId"
                    ORDER BY h.timestamp DESC LIMIT 1
                ), 0),
                'controllerId', COALESCE((
                    SELECT r."deviceControllerId" FROM relay_channels rc
                    JOIN relays r ON r.id = rc."relayId"
                    WHERE rc."deviceId" = d."deviceId"
                      AND rc."deletedAt" IS NULL AND r."deletedAt" IS NULL
                    LIMIT 1
                ), d."controllerId"),
                'usage', jsonb_build_object('totalHours', 0, 'states', '{{}}'::jsonb)
            )
            FROM devices d WHERE
                d."deletedAt" IS NULL
                AND ($1 OR d.status::text = 'approved')
                AND ($2 = '' OR d."deviceName" ILIKE '%' || $2 || '%'
                    OR d."deviceId" ILIKE '%' || $2 || '%'
                    OR d."deviceType" ILIKE '%' || $2 || '%'
                    OR d."macAddress" ILIKE '%' || $2 || '%')
                AND ($3 = '' OR lower($4) = 'all' OR CASE $3
                    WHEN 'deviceType' THEN d."deviceType" ILIKE '%' || $4 || '%'
                    WHEN 'deviceFamily' THEN d."deviceFamily" ILIKE '%' || $4 || '%'
                    WHEN 'status' THEN d.status::text ILIKE '%' || $4 || '%'
                    WHEN 'state' THEN d.state::text ILIKE '%' || $4 || '%'
                    ELSE TRUE END)
            ORDER BY CASE WHEN d.status::text = 'requested' THEN 0 ELSE 1 END,
                CASE WHEN $5 = 'createdAt' AND $6 THEN d."createdAt" END DESC,
                CASE WHEN $5 = 'createdAt' AND NOT $6 THEN d."createdAt" END ASC,
                CASE WHEN $5 = 'updatedAt' AND $6 THEN d."updatedAt" END DESC,
                CASE WHEN $5 = 'updatedAt' AND NOT $6 THEN d."updatedAt" END ASC,
                CASE WHEN $5 = 'deviceName' AND $6 THEN d."deviceName" END DESC,
                CASE WHEN $5 = 'deviceName' AND NOT $6 THEN d."deviceName" END ASC,
                CASE WHEN $5 = 'status' AND $6 THEN d.status::text END DESC,
                CASE WHEN $5 = 'status' AND NOT $6 THEN d.status::text END ASC
            LIMIT $7 OFFSET $8
            "#,
        )
        .bind(is_admin)
        .bind(search)
        .bind(filter_by)
        .bind(filter)
        .bind(sort_by)
        .bind(descending)
        .bind(i64::from(limit))
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let requested_count: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM devices WHERE "deletedAt" IS NULL AND status::text = 'requested'"#,
        )
        .fetch_one(&self.pool)
        .await?;
        let state_rows: Vec<(String, i64)> = sqlx::query_as(
            r#"SELECT state::text, count(*) FROM devices
               WHERE "deletedAt" IS NULL AND status::text = 'approved' GROUP BY state"#,
        )
        .fetch_all(&self.pool)
        .await?;
        let mut device_timers = BTreeMap::new();
        let mut device_timeouts = BTreeMap::new();
        for row in &rows {
            if let Some(device_id) = row.get("deviceId").and_then(serde_json::Value::as_str) {
                device_timers.insert(
                    device_id.to_owned(),
                    row.get("timeout")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                );
                device_timeouts.insert(
                    device_id.to_owned(),
                    row.get("heartbeatTimer")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                );
            }
        }
        Ok(DeviceList {
            data: rows,
            total_pages: u64::try_from(total_devices)
                .unwrap_or(0)
                .div_ceil(u64::from(limit)),
            current_page: page,
            total_devices: u64::try_from(total_devices).unwrap_or(0),
            requested_count: u64::try_from(requested_count).unwrap_or(0),
            device_timers,
            device_timeouts,
            state_count: state_rows
                .into_iter()
                .map(|(state, count)| (state, u64::try_from(count).unwrap_or(0)))
                .collect(),
        })
    }

    async fn device_action_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceActionTarget>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, (String, String, String)>(
            r#"SELECT "deviceId", "ipAddress", status::text
               FROM devices WHERE "deviceId" = $1 AND "deletedAt" IS NULL"#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?
        .map(|(device_id, ip_address, status)| DeviceActionTarget {
            device_id,
            ip_address,
            status,
        }))
    }

    async fn apply_device_action(
        &self,
        device_id: &str,
        action: DeviceAction,
        user_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let device = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(d) FROM devices d
               WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL FOR UPDATE"#,
        )
        .bind(device_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(mut device) = device else {
            transaction.rollback().await?;
            return Ok(None);
        };
        match action {
            DeviceAction::Declined => {
                sqlx::query(
                    r#"INSERT INTO log_entries
                          (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                       VALUES ('device', $1,
                               jsonb_build_object('action', 'declined', 'deviceData', $2,
                                                  'timestamp', now()),
                               'info', now(), now(), now())"#,
                )
                .bind(device_id)
                .bind(&device)
                .execute(&mut *transaction)
                .await?;
                sqlx::query(r#"DELETE FROM device_interfaces WHERE "deviceId" = $1"#)
                    .bind(device_id)
                    .execute(&mut *transaction)
                    .await?;
                sqlx::query(r#"DELETE FROM devices WHERE "deviceId" = $1"#)
                    .bind(device_id)
                    .execute(&mut *transaction)
                    .await?;
            }
            DeviceAction::Approved => {
                let updated = sqlx::query_scalar::<_, serde_json::Value>(
                    r#"UPDATE devices SET status = 'approved', state = 'free',
                                                     "updatedBy" = $2,
                                                     "updatedAt" = now(), "stateUpdatedAt" = now()
                       WHERE "deviceId" = $1 AND status::text = 'requested'
                         AND "deletedAt" IS NULL
                       RETURNING to_jsonb(devices)"#,
                )
                .bind(device_id)
                .bind(user_id)
                .fetch_optional(&mut *transaction)
                .await?;
                let Some(updated) = updated else {
                    return Err(DeviceRepositoryError::Validation(
                        "Device is no longer awaiting approval".to_owned(),
                    ));
                };
                device = updated;
                sqlx::query(
                    r#"INSERT INTO device_state_change
                          ("deviceId", state, "changedAt", "createdAt", "updatedAt")
                       VALUES ($1, 'free', now(), now(), now())"#,
                )
                .bind(device_id)
                .execute(&mut *transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        Ok(Some(device))
    }

    async fn list_controllers(
        &self,
        query: &ControllerListQuery,
    ) -> Result<ControllerList, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(20).clamp(1, MAX_PAGE_SIZE);
        let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
        let search = query.search.as_deref().unwrap_or("").trim();
        let status = query.status.as_deref().unwrap_or("").trim();
        let controller_state = query.state.as_deref().unwrap_or("").trim();
        let sort_by = match query.sort_by.as_deref() {
            Some(
                "updatedAt" | "name" | "deviceFamily" | "macAddress" | "ipAddress" | "status"
                | "state",
            ) => query.sort_by.as_deref().unwrap_or("createdAt"),
            _ => "createdAt",
        };
        let descending = query.desc.unwrap_or(true);
        let total_count: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM device_controllers dc
               WHERE dc."deletedAt" IS NULL
                 AND ($1 = '' OR lower($1) = 'all' OR dc.status::text ILIKE '%' || $1 || '%')
                 AND ($2 = '' OR lower($2) = 'all' OR dc.state::text ILIKE '%' || $2 || '%')
                 AND ($3 = '' OR dc."deviceControllerId" ILIKE '%' || $3 || '%'
                    OR dc.name ILIKE '%' || $3 || '%' OR dc."macAddress" ILIKE '%' || $3 || '%'
                    OR dc."ipAddress" ILIKE '%' || $3 || '%')"#,
        )
        .bind(status)
        .bind(controller_state)
        .bind(search)
        .fetch_one(&self.pool)
        .await?;
        let data = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(dc) || jsonb_build_object('relays', COALESCE((
                    SELECT jsonb_agg(to_jsonb(r) || jsonb_build_object('relayChannels', COALESCE((
                        SELECT jsonb_agg(to_jsonb(rc) || jsonb_build_object('device', CASE
                            WHEN d."deviceId" IS NULL THEN NULL ELSE jsonb_build_object(
                                'deviceId', d."deviceId", 'macAddress', d."macAddress",
                                'deviceName', d."deviceName", 'deviceType', d."deviceType",
                                'ipAddress', d."ipAddress") END) ORDER BY rc."channelNumber")
                        FROM relay_channels rc LEFT JOIN devices d ON d."deviceId" = rc."deviceId"
                        WHERE rc."relayId" = r.id AND rc."deletedAt" IS NULL
                    ), '[]'::jsonb)) ORDER BY r."createdAt")
                    FROM relays r WHERE r."deviceControllerId" = dc."deviceControllerId"
                      AND r."deletedAt" IS NULL
                ), '[]'::jsonb))
               FROM device_controllers dc
               WHERE dc."deletedAt" IS NULL
                 AND ($1 = '' OR lower($1) = 'all' OR dc.status::text ILIKE '%' || $1 || '%')
                 AND ($2 = '' OR lower($2) = 'all' OR dc.state::text ILIKE '%' || $2 || '%')
                 AND ($3 = '' OR dc."deviceControllerId" ILIKE '%' || $3 || '%'
                    OR dc.name ILIKE '%' || $3 || '%' OR dc."macAddress" ILIKE '%' || $3 || '%'
                    OR dc."ipAddress" ILIKE '%' || $3 || '%')
               ORDER BY
                 CASE WHEN $4 = 'createdAt' AND $5 THEN dc."createdAt" END DESC,
                 CASE WHEN $4 = 'createdAt' AND NOT $5 THEN dc."createdAt" END ASC,
                 CASE WHEN $4 = 'updatedAt' AND $5 THEN dc."updatedAt" END DESC,
                 CASE WHEN $4 = 'updatedAt' AND NOT $5 THEN dc."updatedAt" END ASC,
                 CASE WHEN $4 = 'name' AND $5 THEN dc.name END DESC,
                 CASE WHEN $4 = 'name' AND NOT $5 THEN dc.name END ASC,
                 CASE WHEN $4 = 'deviceFamily' AND $5 THEN dc."deviceFamily" END DESC,
                 CASE WHEN $4 = 'deviceFamily' AND NOT $5 THEN dc."deviceFamily" END ASC,
                 CASE WHEN $4 = 'macAddress' AND $5 THEN dc."macAddress" END DESC,
                 CASE WHEN $4 = 'macAddress' AND NOT $5 THEN dc."macAddress" END ASC,
                 CASE WHEN $4 = 'ipAddress' AND $5 THEN dc."ipAddress" END DESC,
                 CASE WHEN $4 = 'ipAddress' AND NOT $5 THEN dc."ipAddress" END ASC,
                 CASE WHEN $4 = 'status' AND $5 THEN dc.status::text END DESC,
                 CASE WHEN $4 = 'status' AND NOT $5 THEN dc.status::text END ASC,
                 CASE WHEN $4 = 'state' AND $5 THEN dc.state::text END DESC,
                 CASE WHEN $4 = 'state' AND NOT $5 THEN dc.state::text END ASC
               LIMIT $6 OFFSET $7"#,
        )
        .bind(status)
        .bind(controller_state)
        .bind(search)
        .bind(sort_by)
        .bind(descending)
        .bind(i64::from(limit))
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let controller_counts: (i64, i64, i64) = sqlx::query_as(
            r#"SELECT count(*), count(*) FILTER (WHERE state::text = 'active'),
               count(*) FILTER (WHERE state::text IN ('not-reachable', 'not_reachable'))
               FROM device_controllers WHERE "deletedAt" IS NULL"#,
        )
        .fetch_one(&self.pool)
        .await?;
        let relay_counts: (i64, i64) = sqlx::query_as(
            r#"SELECT count(*), count(*) FILTER (WHERE state::text = 'connected')
               FROM relays WHERE "deletedAt" IS NULL"#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(ControllerList {
            data,
            total_pages: u64::try_from(total_count)
                .unwrap_or(0)
                .div_ceil(u64::from(limit)),
            current_page: page,
            total_count: u64::try_from(total_count).unwrap_or(0),
            summary: serde_json::json!({
                "controllers": {"total": controller_counts.0, "active": controller_counts.1, "notReachable": controller_counts.2},
                "relays": {"total": relay_counts.0, "connected": relay_counts.1, "disconnected": (relay_counts.0 - relay_counts.1).max(0)}
            }),
        })
    }

    async fn list_user_devices(
        &self,
        query: &UserDeviceListQuery,
    ) -> Result<DeviceList, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(10).clamp(1, MAX_PAGE_SIZE);
        let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
        let search = query.search.as_deref().unwrap_or("").trim();
        let family = query.device_family.as_deref().unwrap_or("").trim();
        let state_filter = query.filter_by.as_deref().unwrap_or("").trim();
        let show_all = query.show_all.unwrap_or(false);
        let sort_by = match query.sort_by.as_deref() {
            Some(
                "updatedAt" | "deviceName" | "status" | "softwareVersion" | "lastTestExecution",
            ) => query.sort_by.as_deref().unwrap_or("createdAt"),
            _ => "createdAt",
        };
        let descending = query.desc.unwrap_or(true);
        let total_devices: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM devices d WHERE d."deletedAt" IS NULL
               AND d.status::text = 'approved'
               AND ($1 = '' OR upper($1) = 'ALL' OR d."deviceFamily" ILIKE '%' || $1 || '%')
               AND ($2 = '' OR lower($2) = 'all' OR d.state::text = $2)
               AND ($3 = '' OR d."deviceName" ILIKE '%' || $3 || '%'
                    OR d."softwareVersion" ILIKE '%' || $3 || '%')
               AND ($4 OR d.state::text IN ('free', 'busy') OR (
                    d."lastTestExecution" IS NOT NULL
                    AND COALESCE(d."lastExecutionStatus"::text, '') NOT IN ('failed', 'completed', 'cancelled')
                    AND d.state::text <> 'unknown'))"#,
        )
        .bind(family).bind(state_filter).bind(search).bind(show_all)
        .fetch_one(&self.pool).await?;
        let data = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(d) || jsonb_build_object(
                'interfaces', COALESCE((SELECT jsonb_agg(to_jsonb(di) ORDER BY di.id)
                    FROM device_interfaces di WHERE di."deviceId" = d."deviceId"), '[]'::jsonb),
                'testExecutions', COALESCE((SELECT jsonb_agg(to_jsonb(te) || jsonb_build_object(
                    'testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id)
                        FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb)))
                    FROM (SELECT * FROM test_executions WHERE "deviceId" = d."deviceId"
                        ORDER BY "updatedAt" DESC LIMIT 1) te), '[]'::jsonb))
               FROM devices d WHERE d."deletedAt" IS NULL AND d.status::text = 'approved'
               AND ($1 = '' OR upper($1) = 'ALL' OR d."deviceFamily" ILIKE '%' || $1 || '%')
               AND ($2 = '' OR lower($2) = 'all' OR d.state::text = $2)
               AND ($3 = '' OR d."deviceName" ILIKE '%' || $3 || '%'
                    OR d."softwareVersion" ILIKE '%' || $3 || '%')
               AND ($4 OR d.state::text IN ('free', 'busy') OR (
                    d."lastTestExecution" IS NOT NULL
                    AND COALESCE(d."lastExecutionStatus"::text, '') NOT IN ('failed', 'completed', 'cancelled')
                    AND d.state::text <> 'unknown'))
               ORDER BY
                 CASE WHEN $5 IN ('createdAt') AND $6 THEN d."createdAt" END DESC,
                 CASE WHEN $5 IN ('createdAt') AND NOT $6 THEN d."createdAt" END ASC,
                 CASE WHEN $5 IN ('updatedAt', 'lastTestExecution') AND $6 THEN d."updatedAt" END DESC,
                 CASE WHEN $5 IN ('updatedAt', 'lastTestExecution') AND NOT $6 THEN d."updatedAt" END ASC,
                 CASE WHEN $5 = 'deviceName' AND $6 THEN d."deviceName" END DESC,
                 CASE WHEN $5 = 'deviceName' AND NOT $6 THEN d."deviceName" END ASC,
                 CASE WHEN $5 = 'status' AND $6 THEN d.status::text END DESC,
                 CASE WHEN $5 = 'status' AND NOT $6 THEN d.status::text END ASC,
                 CASE WHEN $5 = 'softwareVersion' AND $6 THEN d."softwareVersion" END DESC,
                 CASE WHEN $5 = 'softwareVersion' AND NOT $6 THEN d."softwareVersion" END ASC
               LIMIT $7 OFFSET $8"#,
        )
        .bind(family).bind(state_filter).bind(search).bind(show_all)
        .bind(sort_by).bind(descending).bind(i64::from(limit)).bind(offset)
        .fetch_all(&self.pool).await?;
        Ok(DeviceList {
            data,
            total_pages: u64::try_from(total_devices)
                .unwrap_or(0)
                .div_ceil(u64::from(limit)),
            current_page: page,
            total_devices: u64::try_from(total_devices).unwrap_or(0),
            requested_count: 0,
            device_timers: BTreeMap::new(),
            device_timeouts: BTreeMap::new(),
            state_count: BTreeMap::new(),
        })
    }

    async fn active_devices(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(d) || jsonb_build_object(
                'interfaces', COALESCE((SELECT jsonb_agg(to_jsonb(di) ORDER BY di.id)
                    FROM device_interfaces di WHERE di."deviceId" = d."deviceId"), '[]'::jsonb))
               FROM devices d
               WHERE d."deletedAt" IS NULL AND d.status::text = 'approved'
               ORDER BY d."createdAt" DESC"#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn update_heartbeat_timeout(
        &self,
        device_id: &str,
        value: i64,
    ) -> Result<bool, DeviceRepositoryError> {
        Ok(sqlx::query(
            r#"UPDATE devices SET "heartbeatTimer" = $2, "updatedAt" = now()
               WHERE "deviceId" = $1 AND "deletedAt" IS NULL"#,
        )
        .bind(device_id)
        .bind(value)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    async fn edit_controller(
        &self,
        controller_id: &str,
        request: &ControllerEditRequest,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        controller_store::edit(&self.pool, controller_id, request).await
    }

    async fn controller_delete_target(
        &self,
        controller_id: &str,
    ) -> Result<Option<ControllerDeleteTarget>, DeviceRepositoryError> {
        controller_store::delete_target(&self.pool, controller_id).await
    }

    async fn delete_controller(&self, controller_id: &str) -> Result<bool, DeviceRepositoryError> {
        controller_store::delete(&self.pool, controller_id).await
    }

    async fn device_delete_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceDeleteTarget>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, DeviceDeleteTarget>(
            r#"SELECT d."ipAddress" AS ip_address, d."deviceFamily" AS device_family,
                 COALESCE(r."deviceControllerId", d."controllerId") AS controller_id,
                 dc."ipAddress" AS controller_ip, r."serialNumber" AS relay_serial,
                 rc."channelNumber" AS relay_channel
               FROM devices d
               LEFT JOIN relay_channels rc ON rc."deviceId" = d."deviceId" AND rc."deletedAt" IS NULL
               LEFT JOIN relays r ON r.id = rc."relayId" AND r."deletedAt" IS NULL
               LEFT JOIN device_controllers dc ON dc."deviceControllerId" = COALESCE(r."deviceControllerId", d."controllerId")
               WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL
               ORDER BY r."updatedAt" DESC NULLS LAST LIMIT 1"#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn delete_device(&self, device_id: &str) -> Result<bool, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            r#"UPDATE device_controllers controller
               SET mappings = COALESCE((
                       SELECT jsonb_object_agg(entry.key, entry.value)
                       FROM jsonb_each(COALESCE(controller.mappings, '{}'::jsonb)) entry
                       WHERE regexp_replace(lower(entry.key), '[:-]', '', 'g') <> $1
                   ), '{}'::jsonb),
                   "updatedAt" = now()
               WHERE EXISTS (
                   SELECT 1
                   FROM jsonb_each(COALESCE(controller.mappings, '{}'::jsonb)) entry
                   WHERE regexp_replace(lower(entry.key), '[:-]', '', 'g') = $1
               )"#,
        )
        .bind(device_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(r#"DELETE FROM device_interfaces WHERE "deviceId" = $1"#)
            .bind(device_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(r#"DELETE FROM relay_channels WHERE "deviceId" = $1"#)
            .bind(device_id)
            .execute(&mut *transaction)
            .await?;
        let deleted = sqlx::query(r#"DELETE FROM devices WHERE "deviceId" = $1"#)
            .bind(device_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            > 0;
        transaction.commit().await?;
        Ok(deleted)
    }

    async fn builds_for_device_type(
        &self,
        device_type: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT jsonb_build_object(
                 'id', r.id, 'version', r.version, 'status', r.status,
                 'createdAt', r."createdAt", 'lastScannedAt', r."lastScannedAt",
                 'isFaulty', r."isFaulty")
               FROM releases r
               JOIN device_type_folders f ON r."folderName" ILIKE f."folderName"
               WHERE f."deviceType" = $1
               ORDER BY r."createdAt" DESC"#,
        )
        .bind(device_type)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn configure_artifacts(
        &self,
        entries: &[DeviceTypeFolder],
    ) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let mut configured = Vec::with_capacity(entries.len());
        for entry in entries {
            let row = sqlx::query_as::<_, (String, String, String, String)>(
                r#"INSERT INTO device_type_folders
                     ("deviceType", "folderName", "deviceFamily", "defaultVersion")
                   VALUES ($1, $2, $3, $4)
                   ON CONFLICT ("deviceType") DO UPDATE SET
                     "folderName" = EXCLUDED."folderName",
                     "defaultVersion" = EXCLUDED."defaultVersion"
                   RETURNING "deviceType", "folderName", "deviceFamily", "defaultVersion""#,
            )
            .bind(&entry.device_type)
            .bind(&entry.folder_name)
            .bind(&entry.device_family)
            .bind(&entry.default_version)
            .fetch_one(&mut *transaction)
            .await?;
            configured.push(DeviceTypeFolder {
                device_type: row.0,
                folder_name: row.1,
                device_family: row.2,
                default_version: row.3,
            });
        }
        transaction.commit().await?;
        Ok(configured)
    }

    async fn artifact_folders(&self) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, (String, String, String, String)>(
            r#"SELECT "deviceType", "folderName", "deviceFamily", "defaultVersion"
               FROM device_type_folders ORDER BY "deviceType""#,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| DeviceTypeFolder {
            device_type: row.0,
            folder_name: row.1,
            device_family: row.2,
            default_version: row.3,
        })
        .collect())
    }

    async fn artifact_folder_for_device(
        &self,
        device_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, String>(
            r#"SELECT f."folderName" FROM devices d
               JOIN device_type_folders f ON f."deviceType" = d."deviceType"
               WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL"#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn device_reboot_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceRebootTarget>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, DeviceRebootTarget>(
                        r#"SELECT d."deviceFamily" AS device_family,
                                 mapping."ipAddress" AS controller_ip,
                                 mapping.power_port
                             FROM devices d
                             LEFT JOIN LATERAL (
                                 SELECT dc."ipAddress",
                                     dc.mappings -> regexp_replace(lower(d."macAddress"), '[:-]', '', 'g') ->> 'power' AS power_port
                                 FROM device_controllers dc
                                 WHERE dc.state::text = 'active' AND dc.status::text = 'approved'
                                     AND dc."deletedAt" IS NULL
                                     AND dc.mappings ? regexp_replace(lower(d."macAddress"), '[:-]', '', 'g')
                                 ORDER BY dc."updatedAt" DESC LIMIT 1
                             ) mapping ON true
                             WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL"#,
                )
                .bind(device_id)
                .fetch_optional(&self.pool)
                .await?)
    }

    async fn device_power_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError> {
        relay_store::device_power_target(&self.pool, device_id).await
    }

    async fn apply_device_power(
        &self,
        device_id: &str,
        channel_id: Option<Uuid>,
        state: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        relay_store::apply_device_power(&self.pool, device_id, channel_id, state).await
    }

    async fn relays_for_controller(
        &self,
        controller_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        relay_store::relays_for_controller(&self.pool, controller_id).await
    }

    async fn channels_for_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
        relay_store::channels_for_relay(&self.pool, relay_id).await
    }

    async fn available_relay_devices(
        &self,
        query: &AvailableRelayDevicesQuery,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        relay_store::available_devices(&self.pool, query).await
    }

    async fn relay_state_target(
        &self,
        device_id: &str,
    ) -> Result<Option<RelayStateTarget>, DeviceRepositoryError> {
        relay_store::state_target(&self.pool, device_id).await
    }

    async fn update_relay_state(
        &self,
        channel_id: Uuid,
        state: &str,
    ) -> Result<(), DeviceRepositoryError> {
        relay_store::update_state(&self.pool, channel_id, state).await
    }

    async fn relay_identity_target(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError> {
        relay_store::identity_target(&self.pool, relay_id).await
    }

    async fn update_relay_identity(
        &self,
        relay_id: Uuid,
        old_serial: &str,
        serial_number: &str,
        vendor_id: &str,
        product_id: &str,
    ) -> Result<bool, DeviceRepositoryError> {
        relay_store::update_identity(
            &self.pool,
            relay_id,
            old_serial,
            serial_number,
            vendor_id,
            product_id,
        )
        .await
    }

    async fn configure_relay_channels(
        &self,
        configurations: &[RelayChannelConfiguration],
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        relay_configuration_store::configure(&self.pool, configurations).await
    }

    async fn confirm_relay_configuration(
        &self,
        mac: &str,
        succeeded: bool,
    ) -> Result<RelayConfirmationResponse, DeviceRepositoryError> {
        relay_configuration_store::confirm(&self.pool, mac, succeeded).await
    }

    async fn legacy_relays(
        &self,
        query: &LegacyRelayListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        legacy_relay_store::list_relays(&self.pool, query).await
    }

    async fn legacy_relay_channels(
        &self,
        query: &LegacyRelayChannelListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        legacy_relay_store::list_channels(&self.pool, query).await
    }

    async fn legacy_relay_by_id(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        legacy_relay_store::relay_by_id(&self.pool, relay_id).await
    }

    async fn legacy_relay_channel_by_id(
        &self,
        channel_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        legacy_relay_store::channel_by_id(&self.pool, channel_id).await
    }

    async fn delete_legacy_relay(&self, relay_id: Uuid) -> Result<bool, DeviceRepositoryError> {
        legacy_relay_store::delete_relay(&self.pool, relay_id).await
    }

    async fn delete_legacy_relay_channel(
        &self,
        channel_id: Uuid,
    ) -> Result<bool, DeviceRepositoryError> {
        legacy_relay_store::delete_channel(&self.pool, channel_id).await
    }

    async fn configure_legacy_relay(
        &self,
        request: &LegacyRelayConfiguration,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        legacy_relay_store::configure(&self.pool, request).await
    }

    async fn fresh_legacy_relay(&self, relay_id: Uuid) -> Result<(), DeviceRepositoryError> {
        legacy_relay_store::fresh(&self.pool, relay_id).await
    }

    async fn remap_legacy_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        legacy_relay_store::remap(&self.pool, relay_id).await
    }

    async fn legacy_relay_conflicts(
        &self,
        query: &LegacyRelayConflictQuery,
    ) -> Result<RelayConflictState, DeviceRepositoryError> {
        legacy_relay_store::check_conflicts(&self.pool, query).await
    }

    async fn device_state_analytics(&self) -> Result<DeviceStateAnalytics, DeviceRepositoryError> {
        analytics_store::device_state_counts(&self.pool).await
    }

    async fn detailed_device_state_analytics(
        &self,
    ) -> Result<DeviceStateDetailedAnalytics, DeviceRepositoryError> {
        analytics_store::device_state_details(&self.pool).await
    }

    async fn recent_execution_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::recent_executions(&self.pool, user_id, count).await
    }

    async fn execution_analytics_by_id(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::execution_by_id(&self.pool, test_id).await
    }

    async fn daily_execution_analytics(
        &self,
        user_id: &str,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::execution_daily(&self.pool, user_id, from, to).await
    }

    async fn build_comparison_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::build_comparisons(&self.pool, user_id, count).await
    }

    async fn build_comparison_analytics_by_id(
        &self,
        user_id: &str,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::build_comparison_by_id(&self.pool, user_id, build_id).await
    }

    async fn build_performance_analytics(
        &self,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::build_performance(&self.pool, count).await
    }

    async fn build_performance_analytics_by_id(
        &self,
        build_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        analytics_store::build_performance_by_id(&self.pool, build_id).await
    }

    async fn device_usage_analytics(
        &self,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
        device_id: Option<&str>,
        device_family: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        analytics_store::device_usage(&self.pool, start, end, device_id, device_family).await
    }

    async fn test_execution_analytics(
        &self,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        analytics_store::test_execution_analytics(&self.pool, user_id).await
    }

    async fn in_progress_test_analytics(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::in_progress_test_analytics(&self.pool, user_id).await
    }

    async fn test_plan_analytics(
        &self,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        analytics_store::test_plan_summary(&self.pool, user_id).await
    }

    async fn daily_test_analytics(
        &self,
        user_id: &str,
        days: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        analytics_store::daily_test_summary(&self.pool, user_id, days).await
    }

    async fn admin_alerts(
        &self,
        page: i64,
        limit: i64,
        sort_by: &str,
        descending: bool,
    ) -> Result<AlertList, DeviceRepositoryError> {
        alert_store::list_admin(&self.pool, page, limit, sort_by, descending).await
    }

    async fn all_alerts(
        &self,
        page: i64,
        limit: i64,
        from: Option<chrono::DateTime<chrono::Utc>>,
        to: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<AlertList, DeviceRepositoryError> {
        alert_store::list_all(&self.pool, page, limit, from, to).await
    }

    async fn mark_alert_read(&self, alert_id: &str) -> Result<bool, DeviceRepositoryError> {
        alert_store::mark_read(&self.pool, alert_id).await
    }

    async fn mark_all_alerts_read(&self) -> Result<(), DeviceRepositoryError> {
        alert_store::mark_all_read(&self.pool).await
    }

    async fn create_faulty_report(
        &self,
        request: &FaultyReportCreate,
    ) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError> {
        faulty_report_store::create(&self.pool, request).await
    }

    async fn finalize_faulty_report(
        &self,
        report_id: Uuid,
        file_path: Option<&str>,
        logs_path: Option<&str>,
    ) -> Result<repository_types::FaultyReportFinalization, DeviceRepositoryError> {
        faulty_report_store::finalize(&self.pool, report_id, file_path, logs_path).await
    }

    async fn faulty_reports(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        faulty_report_store::list(&self.pool).await
    }

    async fn faulty_report_by_id(
        &self,
        report_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        faulty_report_store::by_id(&self.pool, report_id).await
    }

    async fn update_faulty_report_status(
        &self,
        report_id: Uuid,
        status: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        faulty_report_store::update_status(&self.pool, report_id, status).await
    }

    async fn delete_faulty_report(&self, report_id: Uuid) -> Result<bool, DeviceRepositoryError> {
        faulty_report_store::delete(&self.pool, report_id).await
    }

    async fn record_controller_heartbeat(
        &self,
        heartbeat: &ControllerHeartbeat,
    ) -> Result<ControllerHeartbeatResult, DeviceRepositoryError> {
        heartbeat_store::record_controller(&self.pool, heartbeat).await
    }

    async fn record_device_heartbeat(
        &self,
        device_id: &str,
        heartbeat: &serde_json::Value,
    ) -> Result<Option<DeviceHeartbeatResult>, DeviceRepositoryError> {
        heartbeat_store::record_device(&self.pool, device_id, heartbeat).await
    }
}

#[utoipa::path(get, path = "/api/v1/device/relay/devices/available", tag = "Devices", summary = "List devices available for relay assignment", description = "Returns paginated devices not occupied by another active relay mapping, filtered to the target relay controller generation when known.", params(AvailableRelayDevicesQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated available devices", body = AvailableRelayDevicesResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Available-device query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn available_relay_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AvailableRelayDevicesQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.available_relay_devices(&query).await {
        Ok(devices) => (response_headers, Json(devices)).into_response(),
        Err(error) => repository_error(error, "Failed to list available relay devices"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/relay/controller/{id}", tag = "Devices", summary = "List relays for a controller", description = "Returns each non-deleted relay on the controller with ordered channel and attached-device projections.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Controller relay inventory", body = Vec<RelayRecord>), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Relay query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn relays_for_controller(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(controller_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.relays_for_controller(&controller_id).await {
        Ok(relays) => (response_headers, Json(relays)).into_response(),
        Err(error) => repository_error(error, "Failed to list controller relays"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/relay/channels/relay/{id}", tag = "Devices", summary = "List channels for a relay", description = "Returns ordered non-deleted channel records for one relay, including attached-device summaries.", params(("id" = Uuid, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Relay channel inventory", body = Vec<RelayChannelRecord>), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Relay not found", body = ErrorResponse), (status = 500, description = "Relay channel query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn channels_for_relay(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(relay_id): Path<Uuid>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.channels_for_relay(relay_id).await {
        Ok(Some(channels)) => (response_headers, Json(channels)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Relay not found"),
        Err(error) => repository_error(error, "Failed to list relay channels"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/relay/toggle/{device_id}", tag = "Devices", summary = "Toggle device power", description = "Resolves the device's relay or Gen5 controller power mapping, sends the controller command, then persists and emits the committed power state.", params(("device_id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Updated relay channel or Gen5 power state"), (status = 400, description = "No power mapping exists", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 500, description = "Power-state persistence failed", body = ErrorResponse), (status = 502, description = "Controller rejected the power command", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn toggle_device_power(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.device_power_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => return repository_error(error, "Failed to load device power mapping"),
    };
    let family = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .replace(' ', "")
        .to_lowercase();
    let next_state = if target.current_power.as_deref() == Some("on") {
        "off"
    } else {
        "on"
    };
    let Some(controller_ip) = target.controller_ip.as_deref() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "power_mapping_not_found",
            "Unable to find a mapped controller",
        );
    };
    let (path, payload) = if family == "gen5" {
        let Some(power_port) = target.power_port.as_deref() else {
            return error_response(
                StatusCode::BAD_REQUEST,
                "power_mapping_not_found",
                "No power port mapping was found for this Gen5 device",
            );
        };
        (
            "gen5/power",
            serde_json::json!({"state": next_state, "power": power_port}),
        )
    } else {
        let (Some(serial), Some(channel)) = (target.relay_serial.as_deref(), target.channel_number)
        else {
            return error_response(
                StatusCode::BAD_REQUEST,
                "relay_mapping_not_found",
                "Unable to find a mapped relay",
            );
        };
        (
            "relay",
            serde_json::json!({"serial": serial, "state": next_state, "channel": channel}),
        )
    };
    let url = match reqwest::Url::parse(&format!(
        "http://{controller_ip}:{EDGE_CONTROLLER_PORT}/{path}"
    )) {
        Ok(url) => url,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_controller_address",
                "Controller IP address is invalid",
            );
        }
    };
    match callback_client().post(url).json(&payload).send().await {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => {
            tracing::warn!(status = %response.status(), device_id, "controller rejected power toggle");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "power_toggle_failed",
                "Controller rejected the power command",
            );
        }
        Err(error) => {
            tracing::warn!(%error, device_id, "controller power callback failed");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "controller_unreachable",
                "Controller power endpoint is unavailable",
            );
        }
    }
    match repository
        .apply_device_power(&device_id, target.channel_id, next_state)
        .await
    {
        Ok(Some(mut result)) => {
            let event = device_power_event(&device_id, next_state);
            if let Err(error) = state.event_publisher.publish(event) {
                tracing::warn!(%error, device_id, "failed to publish device power update");
            }
            if family == "gen5" {
                result["power"] = target.power_port.unwrap_or_default().into();
            }
            (response_headers, Json(result)).into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => repository_error(error, "Failed to save device power state"),
    }
}

fn device_power_event(device_id: &str, power: &str) -> ServerEvent {
    ServerEvent {
        event: "device_power_update".to_owned(),
        payload: serde_json::json!({
            "deviceId": device_id,
            "power": power,
            "changedAt": chrono::Utc::now().to_rfc3339(),
        }),
        room: None,
    }
}

#[utoipa::path(get, path = "/api/v1/device/{id}/reboot", tag = "Devices", summary = "Reboot a device", description = "Public compatibility action that resolves the inventory-authorized controller target, requests reboot, and schedules Gen5 recovery tracking when applicable.", params(("id" = String, Path)), responses((status = 200, description = "Device reboot command completed"), (status = 404, description = "Device or power mapping not found", body = ErrorResponse), (status = 500, description = "Reboot target or recovery persistence failed", body = ErrorResponse), (status = 502, description = "Controller rejected the reboot command", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn reboot_device(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.device_reboot_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => return repository_error(error, "Failed to load device reboot configuration"),
    };
    let family = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .replace(' ', "")
        .to_lowercase();
    if matches!(family.as_str(), "gen3" | "gen4") {
        return Json(
            serde_json::json!({"success": true, "message": "Device rebooted successfully"}),
        )
        .into_response();
    }
    if family != "gen5" {
        return error_response(
            StatusCode::BAD_REQUEST,
            "unsupported_device_family",
            "Reboot is not supported for this device family",
        );
    }
    let (Some(controller_ip), Some(power_port)) = (target.controller_ip, target.power_port) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "power_mapping_not_found",
            "No power mapping was found for this Gen5 device",
        );
    };
    let url = match reqwest::Url::parse(&format!(
        "http://{controller_ip}:{EDGE_CONTROLLER_PORT}/reboot-device"
    )) {
        Ok(url) => url,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_controller_address",
                "Controller IP address is invalid",
            );
        }
    };
    match callback_client()
        .post(url)
        .json(&serde_json::json!({"power": power_port}))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => Json(serde_json::json!({
            "success": true, "message": "Device rebooted successfully",
        }))
        .into_response(),
        Ok(response) => {
            tracing::warn!(status = %response.status(), device_id, "controller rejected device reboot");
            error_response(
                StatusCode::BAD_GATEWAY,
                "reboot_failed",
                "Controller rejected the reboot command",
            )
        }
        Err(error) => {
            tracing::warn!(%error, device_id, "controller reboot callback failed");
            error_response(
                StatusCode::BAD_GATEWAY,
                "controller_unreachable",
                "Controller reboot endpoint is unavailable",
            )
        }
    }
}

#[utoipa::path(get, path = "/api/v1/device/{id}/builds", tag = "Devices", summary = "List build versions for a device", description = "Resolves the device type and returns available configured artifact/build version folders.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Build version folders", body = Vec<String>), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 500, description = "Build discovery failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn builds_for_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let folder_name = match repository.artifact_folder_for_device(&device_id).await {
        Ok(Some(folder_name)) => folder_name,
        Ok(None) => return (response_headers, Json(Vec::<String>::new())).into_response(),
        Err(error) => {
            return repository_error(error, "Failed to load device artifact configuration");
        }
    };
    let versions =
        artifacts::discover_versions(&state.config.device.artifacts_base_url, &folder_name).await;
    (response_headers, Json(versions)).into_response()
}

#[utoipa::path(get, path = "/api/v1/device/builds", tag = "Devices", summary = "List releases for a device type", description = "Returns retained release rows matching the requested device type for execution and flashing selection.", params(BuildsQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Matching releases", body = Vec<serde_json::Value>), (status = 400, description = "Device type is missing", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Release query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn builds_for_device_type(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<BuildsQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let device_type = query.device_type.as_deref().unwrap_or("").trim();
    if device_type.is_empty() {
        return (response_headers, Json(Vec::<serde_json::Value>::new())).into_response();
    }
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.builds_for_device_type(device_type).await {
        Ok(builds) => (response_headers, Json(builds)).into_response(),
        Err(error) => repository_error(error, "Failed to list builds"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/config/artifacts", tag = "Devices", summary = "Configure artifact folders", description = "Creates or updates device-type artifact folder metadata used by uploads, scanners, NFS/TFTP staging, and test preparation.", security(("bearer_auth" = [])), request_body = DeviceTypeFolderRequest, responses((status = 200, description = "Artifact folders configured"), (status = 400, description = "Invalid artifact configuration", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Artifact configuration failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn configure_artifacts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<DeviceTypeFolderRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let entries = match request {
        DeviceTypeFolderRequest::One(entry) => vec![entry],
        DeviceTypeFolderRequest::Many(entries) => entries,
    };
    if entries.is_empty()
        || entries.iter().any(|entry| {
            entry.device_type.trim().is_empty()
                || entry.folder_name.trim().is_empty()
                || entry.device_family.trim().is_empty()
                || entry.default_version.trim().is_empty()
        })
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_artifact_configuration",
            "At least one complete artifact configuration is required",
        );
    }
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.configure_artifacts(&entries).await {
        Ok(configured) => (
            response_headers,
            Json(serde_json::json!({
                "message": "Device type folders updated successfully", "data": configured,
            })),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to configure artifacts"),
    }
}

#[utoipa::path(delete, path = "/api/v1/device/{id}", tag = "Devices", summary = "Delete a device", description = "Requests remote device cleanup when reachable, then applies the retained database deletion and committed inventory event behavior.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Device deleted"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 500, description = "Device deletion failed", body = ErrorResponse), (status = 502, description = "Device did not acknowledge deletion", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn delete_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.device_delete_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => return repository_error(error, "Failed to load device"),
    };
    let url = match reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/delete",
        target.ip_address
    )) {
        Ok(url) => url,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_device_address",
                "Device IP address is invalid",
            );
        }
    };
    match callback_client()
        .post(url)
        .json(&serde_json::json!({"UID": device_id}))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => {
            tracing::warn!(status = %response.status(), device_id, "device deletion was rejected");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "device_callback_failed",
                "Device did not acknowledge deletion",
            );
        }
        Err(error) => {
            tracing::warn!(%error, device_id, "device deletion callback failed");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "device_unreachable",
                "Device delete endpoint is unavailable",
            );
        }
    }
    let normalized_family = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .replace(' ', "")
        .to_lowercase();
    if matches!(normalized_family.as_str(), "gen4" | "gen5")
        && let (Some(controller_id), Some(controller_ip)) =
            (&target.controller_id, &target.controller_ip)
    {
        let generation = normalized_family
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u32>()
            .unwrap_or(0);
        if let Ok(url) = reqwest::Url::parse(&format!(
            "http://{controller_ip}:{EDGE_CONTROLLER_PORT}/devCon/delete"
        )) {
            let mut payload =
                serde_json::json!({"uid": controller_id, "gen": generation, "mac": device_id});
            if normalized_family == "gen4" {
                if let Some(serial) = &target.relay_serial {
                    payload["serial"] = serial.clone().into();
                }
                if let Some(channel) = target.relay_channel {
                    payload["channel"] = channel.into();
                }
            }
            if let Err(error) = callback_client().post(url).json(&payload).send().await {
                tracing::warn!(%error, device_id, "controller device-delete notification failed; continuing");
            }
        }
    }
    match repository.delete_device(&device_id).await {
        Ok(true) => (response_headers, Json(serde_json::json!({"success": true, "message": format!("Device {device_id} successfully deleted")}))).into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => repository_error(error, "Failed to delete device"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/controller/{id}", tag = "Devices", summary = "Edit a device controller", description = "Updates mutable controller inventory fields and emits the committed controller-changed event.", params(("id" = String, Path)), security(("bearer_auth" = [])), request_body = ControllerEditRequest, responses((status = 200, description = "Controller updated"), (status = 400, description = "Invalid controller update", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Controller not found", body = ErrorResponse), (status = 500, description = "Controller update failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn edit_controller(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(controller_id): Path<String>,
    Json(request): Json<ControllerEditRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    if request
        .ip_address
        .as_deref()
        .is_some_and(|address| address.trim().is_empty())
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_address",
            "Controller IP address must not be empty",
        );
    }
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.edit_controller(&controller_id, &request).await {
        Ok(Some(controller)) => {
            publish_controller_changed(&state, "updated", &controller_id);
            (
                response_headers,
                Json(serde_json::json!({"success": true, "data": controller})),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Device controller not found",
        ),
        Err(error) => repository_error(error, "Failed to update device controller"),
    }
}

#[utoipa::path(delete, path = "/api/v1/device/controller/{id}", tag = "Devices", summary = "Delete a device controller", description = "Deletes the controller inventory relationship and emits the committed controller-changed event.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 204, description = "Controller deleted"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Controller not found", body = ErrorResponse), (status = 500, description = "Controller deletion failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn delete_controller(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(controller_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.controller_delete_target(&controller_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Device controller not found",
            );
        }
        Err(error) => return repository_error(error, "Failed to load device controller"),
    };
    let generation = target
        .device_family
        .as_deref()
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse::<u32>()
        .unwrap_or(0);
    if let Ok(url) = reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/devCon/delete",
        target.ip_address
    )) && let Err(error) = callback_client()
        .post(url)
        .json(&serde_json::json!({"uid": controller_id, "gen": generation}))
        .send()
        .await
    {
        tracing::warn!(%error, controller_id, "controller delete notification failed; continuing");
    }
    match repository.delete_controller(&controller_id).await {
        Ok(true) => {
            publish_controller_changed(&state, "deleted", &controller_id);
            (response_headers, StatusCode::NO_CONTENT).into_response()
        }
        Ok(false) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Device controller not found",
        ),
        Err(error) => repository_error(error, "Failed to delete device controller"),
    }
}

fn publish_controller_changed(state: &AppState, action: &str, controller_id: &str) {
    if let Err(error) = state.event_publisher.publish(ServerEvent {
        event: "device_controller_changed".to_owned(),
        payload: serde_json::json!({
            "action": action,
            "controllerId": controller_id,
        }),
        room: None,
    }) {
        tracing::warn!(%error, action, controller_id, "failed to publish controller change");
    }
}

#[utoipa::path(get, path = "/api/v1/device/all/active", tag = "Devices", summary = "List approved active devices", description = "Public compatibility route returning complete approved, non-deleted legacy device rows.", responses((status = 200, description = "Approved devices", body = [Object]), (status = 500, description = "Device query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn active_devices(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.active_devices().await {
        Ok(devices) => Json(devices).into_response(),
        Err(error) => repository_error(error, "Failed to list active devices"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/heartbeat/timeout", tag = "Devices", summary = "Configure device heartbeat timeout", description = "Requires the target device to acknowledge the timeout before the new value is persisted.", security(("bearer_auth" = [])), request_body = HeartbeatTimeoutRequest, responses((status = 200, description = "Heartbeat timeout saved", body = CallbackResponse), (status = 400, description = "Timeout or device address is invalid", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 500, description = "Device lookup or timeout persistence failed", body = ErrorResponse), (status = 502, description = "Device rejected or could not receive the timeout", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn update_heartbeat_timeout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<HeartbeatTimeoutRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    if request.value < 0 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_timeout",
            "Heartbeat timeout must not be negative",
        );
    }
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.device_action_target(&request.device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => return repository_error(error, "Failed to load device"),
    };
    let url = match reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/configure/heartbeat",
        target.ip_address,
    )) {
        Ok(url) => url,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_device_address",
                "Device IP address is invalid",
            );
        }
    };
    let response = callback_client()
        .post(url)
        .json(&serde_json::json!({"timeout": request.value}))
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => {
            tracing::warn!(status = %response.status(), device_id = request.device_id, "heartbeat configuration was rejected");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "device_callback_failed",
                "Device did not acknowledge heartbeat configuration",
            );
        }
        Err(error) => {
            tracing::warn!(%error, device_id = request.device_id, "heartbeat configuration callback failed");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "device_unreachable",
                "Device heartbeat endpoint is unavailable",
            );
        }
    }
    match repository.update_heartbeat_timeout(&request.device_id, request.value).await {
        Ok(true) => (response_headers, Json(serde_json::json!({"success": true, "message": "Heartbeat timeout saved successfully."}))).into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => repository_error(error, "Failed to save heartbeat timeout"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/user", tag = "Devices", summary = "List approved devices for users", description = "Returns the approved, non-deleted device inventory projection used by authenticated user workflows.", params(UserDeviceListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated user device inventory", body = DeviceList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Device query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list_user_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<UserDeviceListQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.list_user_devices(&query).await {
        Ok(devices) => (response_headers, Json(devices)).into_response(),
        Err(error) => repository_error(error, "Failed to list user devices"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/controller", tag = "Devices", summary = "List device controllers", description = "Returns paginated controller inventory with retained search, status, family, and sort behavior.", params(ControllerListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated controller inventory", body = ControllerList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Controller query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list_controllers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ControllerListQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.list_controllers(&query).await {
        Ok(controllers) => (response_headers, Json(controllers)).into_response(),
        Err(error) => repository_error(error, "Failed to list device controllers"),
    }
}

#[utoipa::path(put, path = "/api/v1/device/{id}/action", tag = "Devices", summary = "Approve or decline a device request", description = "Approval is committed only after the device acknowledges its expected identity; decline updates persistence directly.", params(("id" = String, Path)), security(("bearer_auth" = [])), request_body = DeviceActionRequest, responses((status = 200, description = "Device request updated", body = DeviceDataResponse), (status = 400, description = "Stored device address is invalid", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Device not found", body = ErrorResponse), (status = 409, description = "Device identity or approval state mismatch", body = ErrorResponse), (status = 500, description = "Device lookup or mutation failed", body = ErrorResponse), (status = 502, description = "Device rejected or could not receive approval", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn device_action(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Json(request): Json<DeviceActionRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let user_id = auth::token_subject(&headers).unwrap_or_else(|| "system".to_owned());
    let target = match repository.device_action_target(&device_id).await {
        Ok(Some(target)) => target,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => return repository_error(error, "Failed to load device"),
    };
    if request.action == DeviceAction::Approved {
        if target.status != "requested" {
            return error_response(
                StatusCode::CONFLICT,
                "invalid_state",
                "Device is no longer awaiting approval",
            );
        }
        let url = match reqwest::Url::parse(&format!(
            "http://{}:{EDGE_CONTROLLER_PORT}/approve",
            target.ip_address
        )) {
            Ok(url) => url,
            Err(_) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_device_address",
                    "Device IP address is invalid",
                );
            }
        };
        let client = callback_client();
        match client
            .post(url)
            .json(&serde_json::json!({"UID": target.device_id, "IP": target.ip_address}))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) if response.status() == reqwest::StatusCode::CONFLICT => {
                return error_response(
                    StatusCode::CONFLICT,
                    "device_identity_mismatch",
                    "Unable to approve device: IP and MAC address do not match the expected device.",
                );
            }
            Ok(response) => {
                tracing::warn!(status = %response.status(), device_id, "device approval was rejected");
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    "device_callback_failed",
                    "Device did not acknowledge approval",
                );
            }
            Err(error) => {
                tracing::warn!(%error, device_id, "device approval callback failed");
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    "device_unreachable",
                    "Device approval endpoint is unavailable",
                );
            }
        }
    }
    match repository
        .apply_device_action(&device_id, request.action, &user_id)
        .await
    {
        Ok(Some(device)) => {
            if request.action == DeviceAction::Approved
                && let Err(error) = state
                    .event_publisher
                    .publish(ServerEvent::alert("device-approval", device.clone()))
            {
                tracing::warn!(%error, device_id, "failed to publish device approval alert");
            }
            (
                response_headers,
                Json(serde_json::json!({"success": true, "data": device})),
            )
                .into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => repository_error(error, "Failed to update device status"),
    }
}

fn callback_client() -> reqwest::Client {
    crate::external_http::client(DEVICE_CALLBACK_TIMEOUT)
        .expect("static device callback HTTP policy is valid")
}

#[utoipa::path(get, path = "/api/v1/device/", tag = "Devices", summary = "List devices", description = "Returns paginated device inventory with retained filters and role-sensitive visibility for frontend configuration views.", params(DeviceListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated device inventory", body = DeviceList), (status = 401, description = "Access token is missing or invalid", body = ErrorResponse), (status = 500, description = "Device query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DeviceListQuery>,
) -> axum::response::Response {
    let (response_headers, is_admin) =
        match auth::authorize_request(&state, &headers, Some("admin")).await {
            Ok(headers) => (headers, true),
            Err(_) => match auth::authorize_request(&state, &headers, None).await {
                Ok(headers) => (headers, false),
                Err(error) => return error.into_response(),
            },
        };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.list_devices(&query, is_admin).await {
        Ok(devices) => (response_headers, Json(devices)).into_response(),
        Err(error) => repository_error(error, "Failed to list devices"),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/device/controller/",
    tag = "Devices",
    summary = "Register an EdgeController",
    description = "Creates or refreshes an EdgeController and atomically replaces its relay inventory. This compatibility route is intentionally public because deployed EdgeControllers do not send bearer tokens.",
    request_body(content = ControllerRegistration, example = json!({
        "macAddress": "aa:bb:cc:dd:ee:ff",
        "ipAddress": "192.0.2.10",
        "deviceFamily": "Gen5",
        "relays": [{"serialNumber": "A100", "state": {"channel_1": 0}}],
        "uid": "aabbccddeeff"
    })),
    responses(
        (status = 201, description = "Controller created", body = ControllerRegistrationResponse),
        (status = 200, description = "Existing controller refreshed", body = ControllerRegistrationResponse),
        (status = 400, description = "Invalid registration", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn register_controller(
    State(state): State<Arc<AppState>>,
    Json(registration): Json<ControllerRegistration>,
) -> axum::response::Response {
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.register_controller(&registration).await {
        Ok(result) => {
            let action = if result.created { "added" } else { "updated" };
            publish_controller_changed(&state, action, &result.controller.device_controller_id);
            if result.created
                && let Err(error) = state.event_publisher.publish(ServerEvent::alert(
                    "device-controller-addition",
                    serde_json::to_value(&result.controller).unwrap_or_default(),
                ))
            {
                tracing::warn!(%error, controller_id = %result.controller.device_controller_id, "failed to publish new-controller alert");
            }
            if let Some(device_family) = result.controller.device_family.as_deref()
                && let Err(error) = crate::workers::sync_controller_ipl(
                    &state,
                    &result.controller.ip_address,
                    device_family,
                )
                .await
            {
                tracing::error!(%error, controller_id = %result.controller.device_controller_id, "failed to synchronize IPL payloads to registered controller");
            }
            if let Ok(url) = reqwest::Url::parse(&format!(
                "http://{}:{EDGE_CONTROLLER_PORT}/confirmation",
                result.controller.ip_address
            )) {
                match callback_client()
                    .post(url)
                    .json(&serde_json::json!({
                        "controllerId": result.controller.device_controller_id,
                    }))
                    .send()
                    .await
                {
                    Ok(response) if response.status().is_success() => {}
                    Ok(response) => tracing::warn!(
                        status = %response.status(),
                        controller_id = %result.controller.device_controller_id,
                        "controller registration confirmation was rejected"
                    ),
                    Err(error) => tracing::warn!(
                        %error,
                        controller_id = %result.controller.device_controller_id,
                        "controller saved but registration confirmation failed"
                    ),
                }
            }
            (
                if result.created {
                    StatusCode::CREATED
                } else {
                    StatusCode::OK
                },
                Json(ControllerRegistrationResponse {
                    success: true,
                    data: result.controller,
                }),
            )
                .into_response()
        }
        Err(error) => repository_error(error, "Failed to register device controller"),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/device/mapping-gen5",
    tag = "Devices",
    summary = "Receive a Gen5 mapping result",
    description = "Public EdgeController callback. Persists the Gen5 UART/power mapping result; the caller IP is taken from the payload or first forwarded address.",
    request_body(content = MappingCallback, example = json!({
        "mac": "aabbccddeeff",
        "status": "success",
        "tty_entry": {"uart": "/dev/ttyUSB0", "power": "1"},
        "ip": "192.0.2.10"
    })),
    responses(
        (status = 200, description = "Mapping callback processed", body = CallbackResponse),
        (status = 400, description = "Invalid callback", body = ErrorResponse),
        (status = 500, description = "Mapping persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn mapping_gen5(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(callback): Json<MappingCallback>,
) -> axum::response::Response {
    let caller_ip = callback.ip.clone().or_else(|| {
        headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .map(str::to_owned)
    });
    let tty_entry = match parse_tty_entry(callback.tty_entry.as_ref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, "validation", &message),
    };
    callback_response(
        &state,
        |repository| async move {
            repository
                .save_gen5_mapping(
                    caller_ip.as_deref().unwrap_or_default(),
                    &callback.mac,
                    callback.status,
                    tty_entry.as_ref(),
                )
                .await
        },
        "Gen5 mapping processed",
    )
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/flash-confirm",
    tag = "Devices",
    summary = "Confirm a Gen5 IPL flash",
    description = "Public EdgeController callback that commits the Gen5 IPL flash outcome before downstream worker processing continues.",
    params(("status" = CallbackStatus, Query, description = "Flash result")),
    responses(
        (status = 200, description = "Confirmation processed", body = CallbackResponse),
        (status = 500, description = "Confirmation persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn flash_confirm(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FlashConfirmationQuery>,
) -> axum::response::Response {
    flash_confirmation_response(
        &state,
        "x5h",
        query.status,
        "Flash confirmation handled successfully",
    )
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/flash-confirm-gen4",
    tag = "Devices",
    summary = "Confirm a Gen4 IPL flash",
    description = "Public EdgeController callback that commits the Gen4 IPL flash outcome before downstream worker processing continues.",
    params(("status" = CallbackStatus, Query, description = "Flash result")),
    responses(
        (status = 200, description = "Confirmation processed", body = CallbackResponse),
        (status = 500, description = "Confirmation persistence failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn flash_confirm_gen4(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FlashConfirmationQuery>,
) -> axum::response::Response {
    flash_confirmation_response(
        &state,
        "v4h",
        query.status,
        "Flash confirmation for Gen4 handled successfully",
    )
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/families",
    tag = "Devices",
    summary = "List device families",
    description = "Returns deterministic distinct non-null device-family values for frontend filters.",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Distinct device families", body = [String]),
        (status = 401, description = "Access token is missing or invalid", body = ErrorResponse),
        (status = 500, description = "Device family query failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn device_families(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    protected_string_list(&state, response_headers, |repository| async move {
        repository.device_families().await
    })
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/deviceTypes",
    tag = "Devices",
    summary = "List device types",
    description = "Returns deterministic distinct device types, optionally filtered by family; the legacy ALL value disables filtering.",
    params(("deviceFamily" = Option<String>, Query, description = "Family filter; ALL returns every type")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Distinct device types", body = [String]),
        (status = 401, description = "Access token is missing or invalid", body = ErrorResponse),
        (status = 500, description = "Device type query failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn device_types(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DeviceTypesQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    protected_string_list(&state, response_headers, |repository| async move {
        repository
            .device_types(query.device_family.as_deref())
            .await
    })
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/device/{id}",
    tag = "Devices",
    summary = "Get a device",
    description = "Returns the stable frontend wrapper around the complete device projection, interfaces, and latest execution context.",
    params(("id" = String, Path, description = "Device identifier")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Device with interfaces and latest execution", body = DeviceDataResponse),
        (status = 404, description = "Device not found", body = ErrorResponse),
        (status = 401, description = "Access token is missing or invalid", body = ErrorResponse),
        (status = 500, description = "Device query failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn device_by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.device_by_id(&device_id).await {
        Ok(Some(device)) => (
            response_headers,
            Json(serde_json::json!({"success": true, "data": device})),
        )
            .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Device not found"),
        Err(error) => repository_error(error, "Failed to get device"),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/device/{id}/heartbeat",
    tag = "Devices",
    summary = "Get the latest device heartbeat",
    description = "Returns the latest non-disconnected heartbeat for a device or controller in the stable success/data wrapper; missing history yields null data.",
    params(("id" = String, Path, description = "Device or controller identifier")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Latest non-disconnected heartbeat, or null", body = DeviceHeartbeatResponse),
        (status = 401, description = "Access token is missing or invalid", body = ErrorResponse),
        (status = 500, description = "Heartbeat query failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn latest_heartbeat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.latest_heartbeat(&device_id).await {
        Ok(heartbeat) => (
            response_headers,
            Json(serde_json::json!({"success": true, "data": heartbeat})),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to get device heartbeat"),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/device/topology",
    tag = "Devices",
    summary = "List devices for topology",
    description = "Returns lightweight non-deleted device nodes in the stable success/data wrapper used by the topology view.",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Lightweight topology nodes", body = DeviceTopologyResponse),
        (status = 401, description = "Access token is missing or invalid", body = ErrorResponse),
        (status = 500, description = "Topology query failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn device_topology(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.topology().await {
        Ok(devices) => (
            response_headers,
            Json(serde_json::json!({"success": true, "data": devices})),
        )
            .into_response(),
        Err(error) => repository_error(error, "Failed to get device topology"),
    }
}

fn repository_error(error: DeviceRepositoryError, message: &str) -> axum::response::Response {
    match error {
        DeviceRepositoryError::Validation(message) => {
            error_response(StatusCode::BAD_REQUEST, "validation", &message)
        }
        DeviceRepositoryError::NotFound(message) => {
            error_response(StatusCode::NOT_FOUND, "not_found", &message)
        }
        DeviceRepositoryError::Conflict(message) => {
            error_response(StatusCode::CONFLICT, "conflict", &message)
        }
        DeviceRepositoryError::Internal(error) => {
            tracing::error!(%error, "device repository operation failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
        }
        DeviceRepositoryError::Database(error) => {
            tracing::error!(%error, "device repository operation failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
        }
    }
}

async fn protected_string_list<F, Fut>(
    state: &AppState,
    response_headers: HeaderMap,
    operation: F,
) -> axum::response::Response
where
    F: FnOnce(Arc<dyn DeviceRepository>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>, DeviceRepositoryError>>,
{
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match operation(repository).await {
        Ok(values) => (response_headers, Json(values)).into_response(),
        Err(error) => repository_error(error, "Device query failed"),
    }
}

async fn callback_response<F, Fut>(
    state: &AppState,
    operation: F,
    message: &'static str,
) -> axum::response::Response
where
    F: FnOnce(Arc<dyn DeviceRepository>) -> Fut,
    Fut: std::future::Future<Output = Result<(), DeviceRepositoryError>>,
{
    let Some(repository) = state.device_repository.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match operation(repository).await {
        Ok(()) => Json(CallbackResponse {
            success: true,
            message,
        })
        .into_response(),
        Err(error) => repository_error(error, "Callback failed"),
    }
}

async fn flash_confirmation_response(
    state: &AppState,
    device_type: &str,
    status: CallbackStatus,
    message: &'static str,
) -> axum::response::Response {
    let Some(repository) = state.device_repository.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.confirm_flash(device_type, status).await {
        Ok(result) => {
            if let Some(execution) = result.cancelled_execution {
                let test_id = execution.test_id;
                let created_by = execution.created_by;
                if let Err(error) = state.event_publisher.publish(ServerEvent {
                    event: "test_execution_update".to_owned(),
                    payload: serde_json::json!({
                        "testId": test_id,
                        "buildId": execution.build_id,
                        "update": {
                            "testId": test_id,
                            "type": "status",
                            "data": {
                                "status": "cancelled",
                                "timestamp": chrono::Utc::now().to_rfc3339(),
                            }
                        },
                        "createdBy": created_by,
                    }),
                    room: Some(format!("user:{created_by}")),
                }) {
                    tracing::warn!(%error, test_id, "failed to publish IPL rejection cancellation");
                }
            }
            if let Some(device_id) = result.freed_device_id
                && let Err(error) = state.event_publisher.publish(ServerEvent {
                    event: "device_state_update".to_owned(),
                    payload: serde_json::json!({
                        "deviceId": device_id,
                        "state": "free",
                        "upgrading": false,
                        "flashing": false,
                        "changedAt": chrono::Utc::now().to_rfc3339(),
                    }),
                    room: None,
                })
            {
                tracing::warn!(%error, device_id, "failed to publish IPL rejection device state");
            }
            Json(CallbackResponse {
                success: true,
                message,
            })
            .into_response()
        }
        Err(error) => repository_error(error, "Callback failed"),
    }
}

fn parse_tty_entry(value: Option<&serde_json::Value>) -> Result<Option<TtyEntry>, String> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => serde_json::from_str(value)
            .map(Some)
            .map_err(|_| "tty_entry must contain a valid JSON mapping".to_owned()),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| "tty_entry must contain uart and power strings".to_owned()),
    }
}

fn error_response(status: StatusCode, code: &str, message: &str) -> axum::response::Response {
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
