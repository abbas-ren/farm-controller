use sqlx::PgPool;
use uuid::Uuid;

use super::{
    CallbackStatus, TtyEntry,
    error::DeviceRepositoryError,
    repository_types::{FlashCancelledExecution, FlashConfirmationResult},
    validation::normalize_mapping_mac,
};

pub(crate) async fn save_gen5_mapping(
    pool: &PgPool,
    caller_ip: &str,
    mac: &str,
    status: CallbackStatus,
    tty_entry: Option<&TtyEntry>,
) -> Result<(), DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let mappings = sqlx::query_scalar::<_, serde_json::Value>(
        r#"
        SELECT mappings FROM device_controllers
        WHERE "ipAddress" = $1
        FOR UPDATE
        "#,
    )
    .bind(caller_ip)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(mappings) = mappings else {
        transaction.commit().await?;
        return Ok(());
    };

    match status {
        CallbackStatus::Success => {
            if let Some(tty_entry) = tty_entry {
                let normalized_mac = normalize_mapping_mac(mac)?;
                let mapping = serde_json::json!({
                    normalized_mac: {"uart": tty_entry.uart, "power": tty_entry.power}
                });
                sqlx::query(
                    r#"
                    UPDATE device_controllers
                    SET mappings = mappings || $2::jsonb, "updatedAt" = now()
                    WHERE "ipAddress" = $1
                    "#,
                )
                .bind(caller_ip)
                .bind(sqlx::types::Json(mapping))
                .execute(&mut *transaction)
                .await?;
            }
            sqlx::query(
                r#"
                UPDATE gen5_mapping_queue SET
                    status = 'completed', "lastError" = NULL,
                    "failureDcIp" = NULL, "baselineMappingCount" = NULL,
                    "nextRetryAt" = NULL, "retryScheduleIndex" = NULL,
                    "updatedAt" = now()
                WHERE id = (
                    SELECT id FROM gen5_mapping_queue
                    WHERE lower("macAddress") = lower($1)
                      AND status IN ('waiting', 'failure_polling')
                    ORDER BY "createdAt" ASC LIMIT 1 FOR UPDATE SKIP LOCKED
                )
                "#,
            )
            .bind(mac)
            .execute(&mut *transaction)
            .await?;
        }
        CallbackStatus::Failure => {
            let baseline_count = mappings
                .as_object()
                .map_or(0_i32, |value| value.len() as i32);
            sqlx::query(
                r#"
                UPDATE gen5_mapping_queue SET
                    status = 'failure_polling', "failureDcIp" = $2,
                    "baselineMappingCount" = $3,
                    "nextRetryAt" = now() + interval '5 minutes',
                    "retryScheduleIndex" = 0,
                    "lastError" = 'Device controller reported mapping failure',
                    "updatedAt" = now()
                WHERE id = (
                    SELECT id FROM gen5_mapping_queue
                    WHERE lower("macAddress") = lower($1) AND status = 'waiting'
                    ORDER BY "createdAt" ASC LIMIT 1 FOR UPDATE SKIP LOCKED
                )
                "#,
            )
            .bind(mac)
            .bind(caller_ip)
            .bind(baseline_count)
            .execute(&mut *transaction)
            .await?;
        }
    }
    transaction.commit().await?;
    Ok(())
}

pub(crate) async fn confirm_flash(
    pool: &PgPool,
    device_type: &str,
    status: CallbackStatus,
) -> Result<FlashConfirmationResult, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let mut result = FlashConfirmationResult::default();
    let job = sqlx::query_as::<_, (Uuid, Option<String>, Option<String>)>(
        r#"
        SELECT id, "targetDeviceId", "testId"
        FROM device_action_queue
        WHERE lower("deviceType") = lower($1)
          AND status = 'waiting' AND "iplFlashConfirm" IS NULL
        ORDER BY "createdAt" ASC
        LIMIT 1 FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(device_type)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some((job_id, device_id, test_id)) = job {
        match status {
            CallbackStatus::Success => {
                sqlx::query(
                    r#"
                    UPDATE device_action_queue
                    SET "iplFlashConfirm" = true, "updatedAt" = now()
                    WHERE id = $1
                    "#,
                )
                .bind(job_id)
                .execute(&mut *transaction)
                .await?;
                if let Some(test_id) = test_id.as_deref() {
                    sqlx::query(
                        r#"INSERT INTO log_entries
                              (type, "referenceId", data, level, timestamp,
                               "createdAt", "updatedAt")
                           VALUES ('test', $1, jsonb_build_object('message', $2::text),
                                   'info', now(), now(), now())"#,
                    )
                    .bind(test_id)
                    .bind(
                        "Got flash confirmation success, waiting for device to come back online...",
                    )
                    .execute(&mut *transaction)
                    .await?;
                }
            }
            CallbackStatus::Failure => {
                sqlx::query(
                    r#"
                    UPDATE device_action_queue
                    SET "iplFlashConfirm" = false, status = 'cancelled', "updatedAt" = now()
                    WHERE id = $1
                    "#,
                )
                .bind(job_id)
                .execute(&mut *transaction)
                .await?;
                if let Some(test_id) = test_id.as_deref() {
                    if let Some((build_id, created_by)) =
                        sqlx::query_as::<_, (String, Option<String>)>(
                            r#"UPDATE test_executions
                           SET status = 'cancelled', "executionPhase" = 'COMPLETE_CANCELLED',
                               "endedAt" = now(), "updatedAt" = now()
                           WHERE "testId" = $1
                             AND status::text NOT IN ('cancelled', 'completed', 'failed')
                           RETURNING "buildId"::text, "createdBy""#,
                        )
                        .bind(test_id)
                        .fetch_optional(&mut *transaction)
                        .await?
                    {
                        result.cancelled_execution = Some(FlashCancelledExecution {
                            test_id: test_id.to_owned(),
                            build_id,
                            created_by: created_by.unwrap_or_else(|| "system".to_owned()),
                        });
                    }
                    sqlx::query(
                        r#"INSERT INTO log_entries
                              (type, "referenceId", data, level, timestamp,
                               "createdAt", "updatedAt")
                           VALUES ('test', $1, jsonb_build_object('message', $2::text),
                                   'info', now(), now(), now())"#,
                    )
                    .bind(test_id)
                    .bind("IPL flash confirmation failed. Test execution cancelled")
                    .execute(&mut *transaction)
                    .await?;
                }
                if let Some(device_id) = device_id {
                    let freed = sqlx::query_scalar::<_, String>(
                        r#"
                        UPDATE devices SET state = 'free', "stateUpdatedAt" = now(),
                                           upgrading = false, flashing = false,
                                           "updatedAt" = now()
                        WHERE "deviceId" = $1
                        RETURNING "deviceId"
                        "#,
                    )
                    .bind(&device_id)
                    .fetch_optional(&mut *transaction)
                    .await?;
                    if freed.is_some() {
                        sqlx::query(
                            r#"INSERT INTO device_state_change
                                  ("deviceId", state, "changedAt", "createdAt", "updatedAt")
                               VALUES ($1, 'free', now(), now(), now())"#,
                        )
                        .bind(&device_id)
                        .execute(&mut *transaction)
                        .await?;
                        result.freed_device_id = Some(device_id);
                    }
                }
            }
        }
    }
    transaction.commit().await?;
    Ok(result)
}
