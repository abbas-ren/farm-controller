use crate::devices::repository_types::{DeferredCancellation, DeviceHeartbeatEvent};

/// Applies heartbeat-confirmed recovery work within the device heartbeat transaction.
pub(super) async fn process_device_post_update(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    device_id: &str,
    nfs_path: Option<&str>,
) -> Result<(Vec<DeviceHeartbeatEvent>, Vec<DeferredCancellation>), sqlx::Error> {
    let mut events = Vec::new();
    let mut deferred_cancellations = Vec::new();
    let fallback = sqlx::query_as::<_, (i32, String, String)>(
        r#"SELECT id, "testId", "fallbackPath" FROM fallback_updates
           WHERE "deviceId" = $1 AND status::text IN ('pending', 'flashing')
           ORDER BY "createdAt" DESC LIMIT 1 FOR UPDATE"#,
    )
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some((fallback_id, test_id, fallback_path)) = fallback {
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
        sqlx::query(fallback_update)
            .bind(fallback_id)
            .execute(&mut **transaction)
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
        sqlx::query(execution_update)
            .bind(&test_id)
            .execute(&mut **transaction)
            .await?;
        events.push(DeviceHeartbeatEvent {
            event: "test_execution_update".to_owned(),
            payload: serde_json::json!({
                "testId": test_id,
                "update": {"type": "phase", "data": {"executionPhase": phase}},
            }),
            room: Some(format!("test:{test_id}")),
        });
    }

    let completed_flash = sqlx::query_scalar::<_, uuid::Uuid>(
        r#"UPDATE device_action_queue SET status = 'completed', "updatedAt" = now()
           WHERE id = (
               SELECT id FROM device_action_queue
               WHERE "targetDeviceId" = $1 AND type::text = 'flash'
                 AND status::text = 'running'
               ORDER BY "createdAt" ASC LIMIT 1 FOR UPDATE
           )
           RETURNING id"#,
    )
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if completed_flash.is_some()
        && let Some((release_id, device_type)) = sqlx::query_as::<_, (String, String)>(
            r#"UPDATE releases release
               SET status = 'passed', "isFaulty" = false, "updatedAt" = now()
               FROM devices device
               WHERE release."testedDeviceId" = $1 AND release.status::text = 'testing'
                 AND device."deviceId" = $1
               RETURNING release.id::text, device."deviceType""#,
        )
        .bind(device_id)
        .fetch_optional(&mut **transaction)
        .await?
    {
        events.push(DeviceHeartbeatEvent {
            event: "release-status-changed".to_owned(),
            payload: serde_json::json!({
                "releaseId": release_id,
                "status": "passed",
                "deviceId": device_id,
                "deviceType": device_type,
            }),
            room: None,
        });
    }

    let flash_confirm = sqlx::query_as::<_, (uuid::Uuid, Option<String>)>(
        r#"SELECT id, "testId" FROM device_action_queue
           WHERE "targetDeviceId" = $1 AND status::text = 'waiting'
             AND "iplFlashConfirm" = true
           ORDER BY "createdAt" ASC LIMIT 1 FOR UPDATE"#,
    )
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some((action_id, test_id)) = flash_confirm {
        let can_requeue = if let Some(test_id) = test_id.as_deref() {
            sqlx::query_scalar::<_, bool>(
                r#"SELECT COALESCE(NOT "cancelRequested", true)
                   FROM test_executions WHERE "testId" = $1 FOR UPDATE"#,
            )
            .bind(test_id)
            .fetch_optional(&mut **transaction)
            .await?
            .unwrap_or(false)
        } else {
            true
        };
        if can_requeue {
            sqlx::query(
                r#"UPDATE device_action_queue
                   SET status = 'queued', mode = 'deviceSpecific',
                       "targetDeviceId" = $2, "updatedAt" = now()
                   WHERE id = $1"#,
            )
            .bind(action_id)
            .bind(device_id)
            .execute(&mut **transaction)
            .await?;
            if let Some(test_id) = test_id.as_deref() {
                sqlx::query(
                    r#"UPDATE test_executions
                       SET "executionPhase" = 'REQUEUE_AFTER_IPL_CONFIRMATION',
                           "updatedAt" = now()
                       WHERE "testId" = $1"#,
                )
                .bind(test_id)
                .execute(&mut **transaction)
                .await?;
            }
        } else if let Some(test_id) = test_id.as_deref()
            && let Some((build_id, created_by)) = sqlx::query_as::<_, (String, Option<String>)>(
                r#"UPDATE test_executions
                       SET status = 'cancelled', "cancelHandled" = true,
                           "executionPhase" = 'COMPLETE_CANCELLED',
                           "startedAt" = COALESCE("startedAt", now()), "endedAt" = now(),
                           "updatedAt" = now()
                       WHERE "testId" = $1
                         AND status::text NOT IN ('cancelled', 'completed', 'failed')
                       RETURNING "buildId"::text, "createdBy""#,
            )
            .bind(test_id)
            .fetch_optional(&mut **transaction)
            .await?
        {
            sqlx::query(
                r#"UPDATE device_action_queue SET status = 'cancelled', "updatedAt" = now()
                   WHERE "testId" = $1
                     AND status::text NOT IN ('completed', 'cancelled')"#,
            )
            .bind(test_id)
            .execute(&mut **transaction)
            .await?;
            sqlx::query(
                r#"INSERT INTO log_entries
                      (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                   VALUES ('test', $1, jsonb_build_object('message', $2::text), 'info',
                           now(), now(), now())"#,
            )
            .bind(test_id)
            .bind("This test has been cancelled successfully")
            .execute(&mut **transaction)
            .await?;
            deferred_cancellations.push(DeferredCancellation {
                test_id: test_id.to_owned(),
                build_id,
                created_by: created_by.unwrap_or_else(|| "system".to_owned()),
            });
        }
        if let Some(test_id) = test_id.as_deref() {
            sqlx::query(
                r#"INSERT INTO log_entries
                      (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                   VALUES ('test', $1, jsonb_build_object('message', $2::text), 'info',
                           now(), now(), now())"#,
            )
            .bind(test_id)
            .bind(if can_requeue {
                format!("Device {device_id} is back online after IPL flashing, continuing the execution")
            } else {
                format!("Device {device_id} is back online after IPL flashing; requeue skipped due to cancellation request")
            })
            .execute(&mut **transaction)
            .await?;
        }
        return Ok((events, deferred_cancellations));
    }

    let running_test = sqlx::query_scalar::<_, String>(
        r#"SELECT "testId" FROM device_action_queue
           WHERE "targetDeviceId" = $1 AND type::text = 'test'
             AND status::text = 'running' AND "testId" IS NOT NULL
           ORDER BY "createdAt" ASC LIMIT 1"#,
    )
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(test_id) = running_test {
        sqlx::query(
            r#"UPDATE test_executions
               SET "executionPhase" = 'START_TEST_EXECUTION', "updatedAt" = now()
               WHERE "testId" = $1 AND COALESCE("cancelRequested", false) = false"#,
        )
        .bind(&test_id)
        .execute(&mut **transaction)
        .await?;
    }
    Ok((events, deferred_cancellations))
}
