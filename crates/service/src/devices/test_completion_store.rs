use sea_orm::{
    ColumnTrait, DatabaseBackend, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    Statement, sea_query::Expr,
};

use crate::persistence::entity::test_execution;

use super::{error::DeviceRepositoryError, repository_types::TestCompletionTarget};

#[derive(Debug, FromQueryResult)]
struct TestCompletionTargetRow {
    status: String,
    device_family: Option<String>,
    mac_address: Option<String>,
    device_type: String,
    build_version: Option<String>,
    build_id: Option<String>,
    created_by: Option<String>,
    gen5_controller_ip: Option<String>,
    uart_port: Option<String>,
    gen4_controller_ip: Option<String>,
    relay_serial: Option<String>,
    relay_channel: Option<i32>,
}

pub(crate) async fn test_completion_target(
    orm: &DatabaseConnection,
    test_id: &str,
    device_id: &str,
) -> Result<Option<TestCompletionTarget>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::test_completion_store",
        "test_completion_target",
    );
    // Complex LATERAL join over controller mappings — keep as Statement.
    Ok(
        TestCompletionTargetRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT COALESCE(test.status::text, 'not_executed') AS status,
                         device."deviceFamily" AS device_family,
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
            [test_id.into(), device_id.into()],
        ))
        .one(orm)
        .await?
        .map(|row| TestCompletionTarget {
            status: row.status,
            device_family: row.device_family,
            mac_address: row.mac_address,
            device_type: row.device_type,
            build_version: row.build_version,
            build_id: row.build_id,
            created_by: row.created_by,
            gen5_controller_ip: row.gen5_controller_ip,
            uart_port: row.uart_port,
            gen4_controller_ip: row.gen4_controller_ip,
            relay_serial: row.relay_serial,
            relay_channel: row.relay_channel,
        }),
    )
}

pub(crate) async fn store_rtos_log_path(
    orm: &DatabaseConnection,
    test_id: &str,
    path: &str,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::test_completion_store",
        "store_rtos_log_path",
    );
    let now = chrono::Utc::now();
    test_execution::Entity::update_many()
        .col_expr(
            test_execution::Column::RtosLogPath,
            Expr::value(Some(path.to_owned())),
        )
        .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
        .filter(test_execution::Column::TestId.eq(test_id))
        .exec(orm)
        .await?;
    Ok(())
}
