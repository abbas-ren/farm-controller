use sqlx::PgPool;

use super::{
    ControllerEditRequest, error::DeviceRepositoryError, repository_types::ControllerDeleteTarget,
};

pub(crate) async fn edit(
    pool: &PgPool,
    controller_id: &str,
    request: &ControllerEditRequest,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM device_controllers
           WHERE "deviceControllerId" = $1 AND "deletedAt" IS NULL)"#,
    )
    .bind(controller_id)
    .fetch_one(&mut *transaction)
    .await?;
    if !exists {
        return Ok(None);
    }
    if request
        .device_family
        .as_deref()
        .map(|family| family.replace(' ', ""))
        == Some("Gen5".to_owned())
    {
        sqlx::query(
            r#"DELETE FROM relay_channels WHERE "relayId" IN
               (SELECT id FROM relays WHERE "deviceControllerId" = $1)"#,
        )
        .bind(controller_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(r#"DELETE FROM relays WHERE "deviceControllerId" = $1"#)
            .bind(controller_id)
            .execute(&mut *transaction)
            .await?;
    }
    let controller = sqlx::query_scalar::<_, serde_json::Value>(
        r#"UPDATE device_controllers SET
             name = COALESCE($2, name),
             "deviceFamily" = COALESCE($3, "deviceFamily"),
             "ipAddress" = COALESCE($4, "ipAddress"),
             "updatedAt" = now()
           WHERE "deviceControllerId" = $1 AND "deletedAt" IS NULL
           RETURNING to_jsonb(device_controllers)"#,
    )
    .bind(controller_id)
    .bind(request.name.as_deref())
    .bind(request.device_family.as_deref())
    .bind(request.ip_address.as_deref())
    .fetch_optional(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(controller)
}

pub(crate) async fn delete_target(
    pool: &PgPool,
    controller_id: &str,
) -> Result<Option<ControllerDeleteTarget>, DeviceRepositoryError> {
    let row = sqlx::query_as::<_, (String, Option<String>)>(
        r#"SELECT "ipAddress", "deviceFamily" FROM device_controllers
           WHERE "deviceControllerId" = $1 AND "deletedAt" IS NULL"#,
    )
    .bind(controller_id)
    .fetch_optional(pool)
    .await?;
    Ok(
        row.map(|(ip_address, device_family)| ControllerDeleteTarget {
            ip_address,
            device_family,
        }),
    )
}

pub(crate) async fn delete(
    pool: &PgPool,
    controller_id: &str,
) -> Result<bool, DeviceRepositoryError> {
    Ok(
        sqlx::query(r#"DELETE FROM device_controllers WHERE "deviceControllerId" = $1"#)
            .bind(controller_id)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}
