use super::*;

impl PostgresDeviceRepository {
    pub(super) async fn request_test_cancellation_inner(
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
        // These phases own asynchronous device/boot completion, so cancellation must be observed there.
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
}
