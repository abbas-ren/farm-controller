use sea_orm::{
    ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, Value,
};
use uuid::Uuid;

use crate::{
    devices::repository_types::{DeferredCancellation, DeferredTestLog},
    socket::types::{farm::ServerEvent, frontend::FrontendMessagePayload},
};

/// Applies heartbeat-confirmed recovery work within the device heartbeat transaction.
/// Complex FOR UPDATE / subquery UPDATE…RETURNING paths stay as raw SQL Statements.
pub(super) async fn process_device_post_update(
    transaction: &impl ConnectionTrait,
    device_id: &str,
    nfs_path: Option<&str>,
) -> Result<(Vec<ServerEvent>, Vec<DeferredCancellation>, Vec<DeferredTestLog>), sea_orm::DbErr>
{
    let _api_trace = crate::observability::ApiTrace::enter("devices::heartbeat_store::state_transition", "process_device_post_update");
    let mut events = Vec::new();
    let mut deferred_cancellations = Vec::new();
    let mut deferred_test_logs = Vec::new();

    #[derive(Debug, FromQueryResult)]
    struct FallbackRow {
        id: i32,
        test_id: String,
        fallback_path: String,
    }
    let fallback = FallbackRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT id, "testId" AS test_id, "fallbackPath" AS fallback_path FROM fallback_updates
           WHERE "deviceId" = $1 AND status::text IN ('pending', 'flashing')
           ORDER BY "createdAt" DESC LIMIT 1 FOR UPDATE"#,
        [device_id.into()],
    ))
    .one(transaction)
    .await?;
    if let Some(FallbackRow {
        id: fallback_id,
        test_id,
        fallback_path,
    }) = fallback
    {
        let matched = nfs_path == Some(fallback_path.as_str());
        let phase = if matched {
            "COMPLETE_SUCCESS"
        } else {
            "COMPLETE_FAILED"
        };
        let fallback_update = if matched {
            r#"UPDATE fallback_updates SET status = 'completed', "updatedAt" = now()
               WHERE id = $1"#
        } else {
            r#"UPDATE fallback_updates SET status = 'failed', "updatedAt" = now()
               WHERE id = $1"#
        };
        transaction
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                fallback_update,
                [fallback_id.into()],
            ))
            .await?;
        let execution_update = if matched {
            r#"UPDATE test_executions SET "executionPhase" = 'COMPLETE_SUCCESS',
                                          "updatedAt" = now()
               WHERE "testId" = $1"#
        } else {
            r#"UPDATE test_executions SET "executionPhase" = 'COMPLETE_FAILED',
                                          "updatedAt" = now()
               WHERE "testId" = $1"#
        };
        transaction
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                execution_update,
                [test_id.clone().into()],
            ))
            .await?;
        events.push(ServerEvent::frontend(
            FrontendMessagePayload::BuildTestExecutionUpdate {
                test_id: test_id.to_owned(),
                build_id: "".to_owned(),
                test_case_id: None,
                status: Some(phase.to_owned()),
            },
        ));
    }

    #[derive(Debug, FromQueryResult)]
    struct CompletedFlash {
        #[allow(dead_code)]
        id: Uuid,
    }
    let completed_flash = CompletedFlash::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"UPDATE device_action_queue SET status = 'completed', "updatedAt" = now()
           WHERE id = (
               SELECT id FROM device_action_queue
               WHERE "targetDeviceId" = $1 AND type::text = 'flash'
                 AND status::text = 'running'
               ORDER BY "createdAt" ASC LIMIT 1 FOR UPDATE
           )
           RETURNING id"#,
        [device_id.into()],
    ))
    .one(transaction)
    .await?;

    if let Some(CompletedFlash { id: _ }) = completed_flash {
        #[derive(Debug, FromQueryResult)]
        struct ReleasePass {
            release_id: String,
            device_type: String,
            version: Option<String>,
            user_id: Option<String>,
        }
        if let Some(ReleasePass {
            release_id,
            device_type,
            version,
            user_id,
        }) = ReleasePass::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"UPDATE releases release
               SET status = 'passed', "isFaulty" = false, "updatedAt" = now()
               FROM devices device
               WHERE release."testedDeviceId" = $1 AND release.status::text = 'testing'
                 AND device."deviceId" = $1
               RETURNING release.id::text AS release_id, device."deviceType" AS device_type,
                         release."version" AS version, release."uploadedBy" AS user_id"#,
            [device_id.into()],
        ))
        .one(transaction)
        .await?
        {
            events.push(ServerEvent::frontend(
                FrontendMessagePayload::BuildStatusChanged {
                    release_id: Uuid::parse_str(&release_id).unwrap(),
                    status: "passed".to_owned(),
                    is_faulty: false,
                    device_id: Some(device_id.to_owned()),
                    device_type,
                    version,
                    user_id: user_id.unwrap_or_default(),
                },
            ));
        }
    }

    #[derive(Debug, FromQueryResult)]
    struct FlashConfirm {
        id: Uuid,
        test_id: Option<String>,
    }
    let flash_confirm = FlashConfirm::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT id, "testId" AS test_id FROM device_action_queue
           WHERE "targetDeviceId" = $1 AND status::text = 'waiting'
             AND "iplFlashConfirm" = true
           ORDER BY "createdAt" ASC LIMIT 1 FOR UPDATE"#,
        [device_id.into()],
    ))
    .one(transaction)
    .await?;
    if let Some(FlashConfirm { id: action_id, test_id }) = flash_confirm {
        let can_requeue = if let Some(test_id) = test_id.as_deref() {
            #[derive(Debug, FromQueryResult)]
            struct CanRequeue {
                value: bool,
            }
            CanRequeue::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT COALESCE(NOT "cancelRequested", true) AS value
                   FROM test_executions WHERE "testId" = $1 FOR UPDATE"#,
                [test_id.into()],
            ))
            .one(transaction)
            .await?
            .map(|row| row.value)
            .unwrap_or(false)
        } else {
            true
        };
        if can_requeue {
            transaction
                .execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    r#"UPDATE device_action_queue
                       SET status = 'queued', mode = 'deviceSpecific',
                           "targetDeviceId" = $2, "updatedAt" = now()
                       WHERE id = $1"#,
                    [action_id.into(), device_id.into()],
                ))
                .await?;
            if let Some(test_id) = test_id.as_deref() {
                transaction
                    .execute_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        r#"UPDATE test_executions
                           SET "executionPhase" = 'REQUEUE_AFTER_IPL_CONFIRMATION',
                               "updatedAt" = now()
                           WHERE "testId" = $1"#,
                        [test_id.into()],
                    ))
                    .await?;
            }
        } else if let Some(test_id) = test_id.as_deref() {
            #[derive(Debug, FromQueryResult)]
            struct CancelledExecution {
                build_id: String,
                created_by: Option<String>,
            }
            if let Some(CancelledExecution {
                build_id,
                created_by,
            }) = CancelledExecution::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"UPDATE test_executions
                       SET status = 'cancelled', "cancelHandled" = true,
                           "executionPhase" = 'COMPLETE_CANCELLED',
                           "startedAt" = COALESCE("startedAt", now()), "endedAt" = now(),
                           "updatedAt" = now()
                       WHERE "testId" = $1
                         AND status::text NOT IN ('cancelled', 'completed', 'failed')
                       RETURNING "buildId"::text AS build_id, "createdBy" AS created_by"#,
                [test_id.into()],
            ))
            .one(transaction)
            .await?
            {
                transaction
                    .execute_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        r#"UPDATE device_action_queue SET status = 'cancelled', "updatedAt" = now()
                           WHERE "testId" = $1
                             AND status::text NOT IN ('completed', 'cancelled')"#,
                        [test_id.into()],
                    ))
                    .await?;
                transaction
                    .execute_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        r#"INSERT INTO log_entries
                              (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                           VALUES ('test', $1, jsonb_build_object('message', $2::text), 'info',
                                   now(), now(), now())"#,
                        [
                            test_id.into(),
                            Value::from("This test has been cancelled successfully"),
                        ],
                    ))
                    .await?;
                deferred_cancellations.push(DeferredCancellation {
                    test_id: test_id.to_owned(),
                    build_id,
                    created_by: created_by.unwrap_or_else(|| "system".to_owned()),
                });
            }
        }
        if let Some(test_id) = test_id.as_deref() {
            let message = if can_requeue {
                format!(
                    "Device {device_id} is back online after IPL flashing, continuing the execution"
                )
            } else {
                format!(
                    "Device {device_id} is back online after IPL flashing; requeue skipped due to cancellation request"
                )
            };
            transaction
                .execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    r#"INSERT INTO log_entries
                          (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                       VALUES ('test', $1, jsonb_build_object('message', $2::text), 'info',
                               now(), now(), now())"#,
                    [test_id.into(), message.clone().into()],
                ))
                .await?;
            deferred_test_logs.push(DeferredTestLog {
                test_id: test_id.to_owned(),
                message,
            });
        }
        return Ok((events, deferred_cancellations, deferred_test_logs));
    }

    #[derive(Debug, FromQueryResult)]
    struct RunningTest {
        test_id: String,
    }
    let running_test = RunningTest::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT "testId" AS test_id FROM device_action_queue
           WHERE "targetDeviceId" = $1 AND type::text = 'test'
             AND status::text = 'running' AND "testId" IS NOT NULL
           ORDER BY "createdAt" ASC LIMIT 1"#,
        [device_id.into()],
    ))
    .one(transaction)
    .await?;
    if let Some(RunningTest { test_id }) = running_test {
        transaction
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"UPDATE test_executions
                   SET "executionPhase" = 'START_TEST_EXECUTION', "updatedAt" = now()
                   WHERE "testId" = $1 AND COALESCE("cancelRequested", false) = false"#,
                [test_id.into()],
            ))
            .await?;
    }
    Ok((events, deferred_cancellations, deferred_test_logs))
}
