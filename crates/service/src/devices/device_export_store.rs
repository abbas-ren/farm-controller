//! Load device + interface rows for CSV export.
//!
//! Used by the device export HTTP handler before [`csv_export::format_devices`].
//! Replaces the Node `DeviceService` export query that joined devices with
//! interfaces and applied search / date filters.

use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    sea_query::{Expr, extension::postgres::PgExpr},
};

use crate::persistence::entity::{device, device_interface};

use super::{
    error::DeviceRepositoryError,
    postgres::json_models::{device_json, interface_json},
    DeviceCsvQuery,
};

/// PURPOSE: Fetch non-deleted devices (optional search/date filters) with nested interfaces.
/// USE: Device CSV export handler; returns JSON shaped like legacy `to_jsonb` device payloads.
pub(crate) async fn export_devices(
    orm: &DatabaseConnection,
    query: &DeviceCsvQuery,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::device_export_store", "export_devices");
    let search = query.search.as_deref().map(str::trim).unwrap_or_default();
    // Soft-delete filter matches inventory list semantics.
    let mut select = device::Entity::find().filter(device::Column::DeletedAt.is_null());
    if !search.is_empty() {
        let pattern = format!("%{search}%");
        select = select.filter(
            Condition::any()
                .add(Expr::col(device::Column::DeviceName).ilike(&pattern))
                .add(Expr::col(device::Column::DeviceId).ilike(&pattern))
                .add(Expr::col(device::Column::DeviceType).ilike(&pattern))
                .add(Expr::col(device::Column::MacAddress).ilike(&pattern)),
        );
    }
    if let Some(from) = query.from_date.as_deref().filter(|v| !v.is_empty()) {
        select = select.filter(Expr::cust_with_values(
            r#""createdAt" >= $1::timestamptz"#,
            [from],
        ));
    }
    if let Some(to) = query.to_date.as_deref().filter(|v| !v.is_empty()) {
        select = select.filter(Expr::cust_with_values(
            r#""createdAt" <= $1::timestamptz"#,
            [to],
        ));
    }
    let devices = select
        .order_by_desc(device::Column::CreatedAt)
        .all(orm)
        .await?;

    let mut rows = Vec::with_capacity(devices.len());
    for device_model in devices {
        let interfaces = device_interface::Entity::find()
            .filter(device_interface::Column::DeviceId.eq(&device_model.device_id))
            .order_by_asc(device_interface::Column::Id)
            .all(orm)
            .await?;
        // Assemble camelCase JSON matching legacy Sequelize to_jsonb(device) + interfaces.
        let mut value = device_json(&device_model);
        value["interfaces"] =
            serde_json::Value::Array(interfaces.iter().map(interface_json).collect());
        rows.push(value);
    }
    Ok(rows)
}
