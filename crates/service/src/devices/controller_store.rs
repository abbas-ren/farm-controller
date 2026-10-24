use sea_orm::{
    ColumnTrait, DatabaseBackend, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    Statement, TransactionTrait, sea_query::Expr,
};

use crate::persistence::entity::{device_controller, relay, relay_channel};

use super::{
    ControllerEditRequest, error::DeviceRepositoryError, repository_types::ControllerDeleteTarget,
};

pub(crate) async fn edit(
    orm: &DatabaseConnection,
    controller_id: &str,
    request: &ControllerEditRequest,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::controller_store", "edit");
    let transaction = orm.begin().await?;
    let exists = device_controller::Entity::find_by_id(controller_id.to_owned())
        .filter(device_controller::Column::DeletedAt.is_null())
        .one(&transaction)
        .await?
        .is_some();
    if !exists {
        return Ok(None);
    }
    if request
        .device_family
        .as_deref()
        .map(|family| family.replace(' ', ""))
        == Some("Gen5".to_owned())
    {
        let relay_ids = relay::Entity::find()
            .filter(relay::Column::DeviceControllerId.eq(controller_id))
            .all(&transaction)
            .await?
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
        if !relay_ids.is_empty() {
            relay_channel::Entity::delete_many()
                .filter(relay_channel::Column::RelayId.is_in(relay_ids))
                .exec(&transaction)
                .await?;
        }
        relay::Entity::delete_many()
            .filter(relay::Column::DeviceControllerId.eq(controller_id))
            .exec(&transaction)
            .await?;
    }
    let now = chrono::Utc::now();
    let mut update = device_controller::Entity::update_many()
        .col_expr(device_controller::Column::UpdatedAt, Expr::value(now))
        .filter(device_controller::Column::DeviceControllerId.eq(controller_id))
        .filter(device_controller::Column::DeletedAt.is_null());
    if let Some(name) = request.name.as_deref() {
        update = update.col_expr(
            device_controller::Column::Name,
            Expr::value(Some(name.to_owned())),
        );
    }
    if let Some(device_family) = request.device_family.as_deref() {
        update = update.col_expr(
            device_controller::Column::DeviceFamily,
            Expr::value(Some(device_family.to_owned())),
        );
    }
    if let Some(ip_address) = request.ip_address.as_deref() {
        update = update.col_expr(
            device_controller::Column::IpAddress,
            Expr::value(ip_address.to_owned()),
        );
    }
    update.exec(&transaction).await?;

    // RETURNING to_jsonb kept for API response shape.
    #[derive(Debug, FromQueryResult)]
    struct ControllerJson {
        value: serde_json::Value,
    }
    let controller = ControllerJson::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(device_controllers) AS value
           FROM device_controllers
           WHERE "deviceControllerId" = $1 AND "deletedAt" IS NULL"#,
        [controller_id.into()],
    ))
    .one(&transaction)
    .await?
    .map(|row| row.value);
    transaction.commit().await?;
    Ok(controller)
}

pub(crate) async fn delete_target(
    orm: &DatabaseConnection,
    controller_id: &str,
) -> Result<Option<ControllerDeleteTarget>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::controller_store", "delete_target");
    Ok(
        device_controller::Entity::find_by_id(controller_id.to_owned())
            .filter(device_controller::Column::DeletedAt.is_null())
            .one(orm)
            .await?
            .map(|row| ControllerDeleteTarget {
                ip_address: row.ip_address,
                device_family: row.device_family,
            }),
    )
}

pub(crate) async fn delete(
    orm: &DatabaseConnection,
    controller_id: &str,
) -> Result<bool, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::controller_store", "delete");
    let result = device_controller::Entity::delete_by_id(controller_id.to_owned())
        .exec(orm)
        .await?;
    Ok(result.rows_affected > 0)
}
