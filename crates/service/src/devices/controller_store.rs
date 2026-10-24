//! Controller edit / delete persistence helpers.
//!
//! Called from `ControllerRepository` Postgres methods and controller HTTP
//! handlers. Ports Node `DeviceService` controller update/delete, including
//! the Gen5 family switch that hard-deletes attached relays and channels.

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait, sea_query::Expr,
};

use crate::persistence::entity::{device_controller, relay, relay_channel};

use super::{
    ControllerEditRequest, error::DeviceRepositoryError, postgres::json_models::controller_json,
    repository_types::ControllerDeleteTarget,
};

/// PURPOSE: Update controller name/family/IP; clear Gen5 relays when family becomes Gen5.
/// USE: Controller edit API; returns updated controller JSON or `None` if missing/soft-deleted.
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
    // Legacy DeviceService: switching to Gen5 removes prior relay wiring for that controller.
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

    let controller = device_controller::Entity::find_by_id(controller_id.to_owned())
        .filter(device_controller::Column::DeletedAt.is_null())
        .one(&transaction)
        .await?
        .map(|model| controller_json(&model));
    transaction.commit().await?;
    Ok(controller)
}

/// PURPOSE: Load controller IP/family needed before outbound delete to the edge.
/// USE: Controller delete handler gathers edge target before hard-deleting the row.
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

/// PURPOSE: Hard-delete a controller row by id.
/// USE: After edge cleanup succeeds in the controller delete handler.
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
