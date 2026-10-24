use super::*;
use super::json_models::{device_json, interface_json, test_execution_json};

use crate::persistence::entity::{
    device, device_controller_heartbeat, device_interface, heartbeat, relay, relay_channel,
    test_execution,
};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, extension::postgres::PgExpr},
};

fn device_list_filters(
    mut select: sea_orm::Select<device::Entity>,
    is_admin: bool,
    search: &str,
    filter_by: &str,
    filter: &str,
) -> sea_orm::Select<device::Entity> {
    select = select.filter(device::Column::DeletedAt.is_null());
    if !is_admin {
        select = select.filter(device::Column::Status.eq(device::DeviceStatus::Approved));
    }
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
    if !filter_by.is_empty() && !filter.eq_ignore_ascii_case("all") && !filter.is_empty() {
        let pattern = format!("%{filter}%");
        select = match filter_by {
            "deviceType" => {
                select.filter(Expr::col(device::Column::DeviceType).ilike(&pattern))
            }
            "deviceFamily" => {
                select.filter(Expr::col(device::Column::DeviceFamily).ilike(&pattern))
            }
            "status" => select.filter(Expr::cust_with_values(
                r#"status::text ILIKE ('%' || $1 || '%')"#,
                [filter],
            )),
            "state" => select.filter(Expr::cust_with_values(
                r#"state::text ILIKE ('%' || $1 || '%')"#,
                [filter],
            )),
            _ => select,
        };
    }
    select
}

async fn controller_id_for_device(
    orm: &sea_orm::DatabaseConnection,
    device_id: &str,
    fallback: Option<String>,
) -> Result<Option<String>, DeviceRepositoryError> {
    let mapped = relay_channel::Entity::find()
        .filter(relay_channel::Column::DeviceId.eq(device_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .find_also_related(relay::Entity)
        .all(orm)
        .await?
        .into_iter()
        .filter_map(|(_, relay)| {
            relay.filter(|r| r.deleted_at.is_none())
                .map(|r| (r.updated_at, r.device_controller_id))
        })
        .max_by_key(|(updated_at, _)| *updated_at)
        .map(|(_, controller_id)| controller_id);
    Ok(mapped.or(fallback))
}

#[async_trait]
impl DeviceInventoryRepository for PostgresDeviceRepository {
    async fn device_families(&self) -> Result<Vec<String>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::inventory",
            "device_families",
        );
        let mut families: Vec<String> = device::Entity::find()
            .filter(device::Column::DeletedAt.is_null())
            .filter(device::Column::DeviceFamily.is_not_null())
            .all(self.orm())
            .await?
            .into_iter()
            .filter_map(|d| d.device_family)
            .collect();
        families.sort();
        families.dedup();
        Ok(families)
    }

    async fn device_types(
        &self,
        device_family: Option<&str>,
    ) -> Result<Vec<String>, DeviceRepositoryError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("devices::postgres::inventory", "device_types");
        let mut select = device::Entity::find()
            .filter(device::Column::DeletedAt.is_null())
            .filter(device::Column::DeviceType.is_not_null());
        if let Some(family) = device_family.filter(|f| *f != "ALL") {
            select = select.filter(device::Column::DeviceFamily.eq(family));
        }
        let mut types: Vec<String> = select
            .all(self.orm())
            .await?
            .into_iter()
            .filter_map(|d| d.device_type)
            .collect();
        types.sort();
        types.dedup();
        Ok(types)
    }

    async fn device_by_id(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("devices::postgres::inventory", "device_by_id");
        let Some(device_model) = device::Entity::find()
            .filter(device::Column::DeviceId.eq(device_id))
            .filter(device::Column::DeletedAt.is_null())
            .one(self.orm())
            .await?
        else {
            return Ok(None);
        };
        let interfaces = device_interface::Entity::find()
            .filter(device_interface::Column::DeviceId.eq(device_id))
            .order_by_asc(device_interface::Column::Id)
            .all(self.orm())
            .await?
            .iter()
            .map(interface_json)
            .collect::<Vec<_>>();
        let test_executions = test_execution::Entity::find()
            .filter(test_execution::Column::DeviceId.eq(device_id))
            .order_by_desc(test_execution::Column::UpdatedAt)
            .limit(1)
            .all(self.orm())
            .await?
            .iter()
            .map(test_execution_json)
            .collect::<Vec<_>>();
        let mut value = device_json(&device_model);
        value["interfaces"] = serde_json::Value::Array(interfaces);
        value["testExecutions"] = serde_json::Value::Array(test_executions);
        Ok(Some(value))
    }

    async fn latest_heartbeat(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::inventory",
            "latest_heartbeat",
        );
        let heartbeats = heartbeat::Entity::find()
            .filter(heartbeat::Column::DeviceId.eq(device_id))
            .order_by_desc(heartbeat::Column::Timestamp)
            .all(self.orm())
            .await?;
        if let Some(hb) = heartbeats.into_iter().find(|hb| {
            hb.data
                .get("status")
                .and_then(|v| v.as_str())
                .is_none_or(|s| s != "disconnected")
        }) {
            return Ok(Some(serde_json::json!({
                "timestamp": hb.timestamp,
                "data": hb.data,
                "timeout": hb.timeout,
            })));
        }
        let controller_heartbeats = device_controller_heartbeat::Entity::find()
            .filter(device_controller_heartbeat::Column::ControllerId.eq(device_id))
            .order_by_desc(device_controller_heartbeat::Column::Timestamp)
            .all(self.orm())
            .await?;
        Ok(controller_heartbeats
            .into_iter()
            .find(|hb| {
                hb.data
                    .get("status")
                    .and_then(|v| v.as_str())
                    .is_none_or(|s| s != "disconnected")
            })
            .map(|hb| {
                serde_json::json!({
                    "timestamp": hb.timestamp,
                    "data": hb.data,
                    "timeout": hb.timeout,
                })
            }))
    }

    async fn topology(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("devices::postgres::inventory", "topology");
        let devices = device::Entity::find()
            .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
            .filter(device::Column::DeletedAt.is_null())
            .order_by_asc(device::Column::DeviceName)
            .order_by_asc(device::Column::DeviceId)
            .all(self.orm())
            .await?;
        let mut rows = Vec::with_capacity(devices.len());
        for device_model in devices {
            let controller_id = controller_id_for_device(
                self.orm(),
                &device_model.device_id,
                device_model.controller_id.clone(),
            )
            .await?;
            rows.push(serde_json::json!({
                "deviceId": device_model.device_id,
                "deviceName": device_model.device_name,
                "deviceType": device_model.device_type,
                "deviceFamily": device_model.device_family,
                "macAddress": device_model.mac_address,
                "ipAddress": device_model.ip_address,
                "state": json_models::device_state_str(device_model.state.as_ref()),
                "status": json_models::device_status_str(device_model.status.as_ref()),
                "controllerId": controller_id,
                "heartbeatTimer": device_model.heartbeat_timer,
            }));
        }
        Ok(rows)
    }

    async fn list_devices(
        &self,
        query: &DeviceListQuery,
        is_admin: bool,
    ) -> Result<DeviceList, DeviceRepositoryError> {
        let _api_trace =
            crate::observability::ApiTrace::enter("devices::postgres::inventory", "list_devices");
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(16).clamp(1, MAX_PAGE_SIZE);
        let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
        let search = query.search.as_deref().unwrap_or("").trim();
        let filter_by = match query.filter_by.as_deref().unwrap_or("") {
            "deviceType" | "deviceFamily" | "status" | "state" => {
                query.filter_by.as_deref().unwrap_or("")
            }
            _ => "",
        };
        let filter = query.filter.as_deref().unwrap_or("").trim();
        let sort_by = match query.sort_by.as_deref() {
            Some("updatedAt" | "deviceName" | "status") => {
                query.sort_by.as_deref().unwrap_or("createdAt")
            }
            _ => "createdAt",
        };
        let descending = query.desc.unwrap_or(true);

        let total_devices = device_list_filters(
            device::Entity::find(),
            is_admin,
            search,
            filter_by,
            filter,
        )
        .count(self.orm())
        .await? as i64;

        let mut select =
            device_list_filters(device::Entity::find(), is_admin, search, filter_by, filter);
        // requested first, then sort
        select = select.order_by_asc(Expr::cust(
            r#"CASE WHEN status::text = 'requested' THEN 0 ELSE 1 END"#,
        ));
        select = match (sort_by, descending) {
            ("updatedAt", true) => select.order_by_desc(device::Column::UpdatedAt),
            ("updatedAt", false) => select.order_by_asc(device::Column::UpdatedAt),
            ("deviceName", true) => select.order_by_desc(device::Column::DeviceName),
            ("deviceName", false) => select.order_by_asc(device::Column::DeviceName),
            ("status", true) => select.order_by_desc(device::Column::Status),
            ("status", false) => select.order_by_asc(device::Column::Status),
            (_, true) => select.order_by_desc(device::Column::CreatedAt),
            (_, false) => select.order_by_asc(device::Column::CreatedAt),
        };
        let devices = select
            .limit(u64::from(limit))
            .offset(offset as u64)
            .all(self.orm())
            .await?;

        let mut rows = Vec::with_capacity(devices.len());
        for device_model in devices {
            let interfaces = device_interface::Entity::find()
                .filter(device_interface::Column::DeviceId.eq(&device_model.device_id))
                .order_by_asc(device_interface::Column::Id)
                .all(self.orm())
                .await?
                .iter()
                .map(interface_json)
                .collect::<Vec<_>>();
            let timeout = heartbeat::Entity::find()
                .filter(heartbeat::Column::DeviceId.eq(&device_model.device_id))
                .order_by_desc(heartbeat::Column::Timestamp)
                .one(self.orm())
                .await?
                .map(|hb| hb.timeout)
                .unwrap_or(0);
            let controller_id = controller_id_for_device(
                self.orm(),
                &device_model.device_id,
                device_model.controller_id.clone(),
            )
            .await?;
            let mut value = device_json(&device_model);
            value["interfaces"] = serde_json::Value::Array(interfaces);
            value["timeout"] = serde_json::json!(timeout);
            value["controllerId"] = serde_json::json!(controller_id);
            value["usage"] = serde_json::json!({"totalHours": 0, "states": {}});
            rows.push(value);
        }

        let requested_count = device::Entity::find()
            .filter(device::Column::DeletedAt.is_null())
            .filter(device::Column::Status.eq(device::DeviceStatus::Requested))
            .count(self.orm())
            .await? as i64;

        let approved = device::Entity::find()
            .filter(device::Column::DeletedAt.is_null())
            .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
            .all(self.orm())
            .await?;
        let mut state_count = BTreeMap::new();
        for device_model in approved {
            if let Some(state) = json_models::device_state_str(device_model.state.as_ref()) {
                *state_count.entry(state.to_owned()).or_insert(0u64) += 1;
            }
        }

        let mut device_timers = BTreeMap::new();
        let mut device_timeouts = BTreeMap::new();
        for row in &rows {
            if let Some(device_id) = row.get("deviceId").and_then(serde_json::Value::as_str) {
                device_timers.insert(
                    device_id.to_owned(),
                    row.get("timeout")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                );
                device_timeouts.insert(
                    device_id.to_owned(),
                    row.get("heartbeatTimer")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                );
            }
        }
        Ok(DeviceList {
            data: rows,
            total_pages: u64::try_from(total_devices)
                .unwrap_or(0)
                .div_ceil(u64::from(limit)),
            current_page: page,
            total_devices: u64::try_from(total_devices).unwrap_or(0),
            requested_count: u64::try_from(requested_count).unwrap_or(0),
            device_timers,
            device_timeouts,
            state_count,
        })
    }
}
