use sqlx::PgPool;

use super::{error::DeviceRepositoryError, repository_types::TestCompletionTarget};

pub(crate) async fn test_completion_target(
    pool: &PgPool,
    test_id: &str,
    device_id: &str,
) -> Result<Option<TestCompletionTarget>, DeviceRepositoryError> {
    Ok(sqlx::query_as::<_, TestCompletionTarget>(
                r#"SELECT test.status::text AS status, device."deviceFamily" AS device_family,
                                    device."macAddress" AS mac_address, test."deviceType" AS device_type,
                                    test."buildVersion" AS build_version, test."buildId"::text AS build_id,
                                    test."createdBy" AS created_by,
                                    gen5."ipAddress" AS gen5_controller_ip, gen5.uart_port,
                                    relay_controller."ipAddress" AS gen4_controller_ip,
                                    relay."serialNumber" AS relay_serial,
                                    relay_channel."channelNumber" AS relay_channel
                     FROM test_executions test
                     LEFT JOIN devices device
                         ON device."deviceId" = $2 AND device."deletedAt" IS NULL
                     LEFT JOIN relay_channels relay_channel
                         ON relay_channel."deviceId" = device."deviceId"
                        AND relay_channel."deletedAt" IS NULL
                     LEFT JOIN relays relay
                         ON relay.id = relay_channel."relayId" AND relay."deletedAt" IS NULL
                     LEFT JOIN device_controllers relay_controller
                         ON relay_controller."deviceControllerId" = relay."deviceControllerId"
                        AND relay_controller."deletedAt" IS NULL
                     LEFT JOIN LATERAL (
                         SELECT controller."ipAddress",
                                        controller.mappings -> regexp_replace(lower(device."macAddress"), '[:-]', '', 'g') ->> 'uart' AS uart_port
                         FROM device_controllers controller
                         WHERE controller.state::text = 'active'
                             AND controller.status::text = 'approved'
                             AND controller."deletedAt" IS NULL
                             AND controller.mappings ? regexp_replace(lower(device."macAddress"), '[:-]', '', 'g')
                         ORDER BY (controller."deviceControllerId" = device."controllerId") DESC,
                                            controller."updatedAt" DESC
                         LIMIT 1
                     ) gen5 ON true
                     WHERE test."testId" = $1
                     ORDER BY relay_channel."updatedAt" DESC NULLS LAST
                     LIMIT 1"#,
    )
    .bind(test_id)
        .bind(device_id)
        .fetch_optional(pool)
    .await?)
}

pub(crate) async fn store_rtos_log_path(
    pool: &PgPool,
    test_id: &str,
    path: &str,
) -> Result<(), DeviceRepositoryError> {
    sqlx::query(
        r#"UPDATE test_executions SET "rtosLogPath" = $2, "updatedAt" = now()
                     WHERE "testId" = $1"#,
    )
    .bind(test_id)
    .bind(path)
    .execute(pool)
    .await?;
    Ok(())
}
