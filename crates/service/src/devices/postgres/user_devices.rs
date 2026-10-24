use super::*;
use super::json_models::{device_json, interface_json, test_case_json, test_execution_json};

use crate::persistence::entity::{device, device_interface, test_case, test_execution};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, extension::postgres::PgExpr},
};

fn user_device_filters(
    mut select: sea_orm::Select<device::Entity>,
    family: &str,
    state_filter: &str,
    search: &str,
    show_all: bool,
) -> sea_orm::Select<device::Entity> {
    select = select
        .filter(device::Column::DeletedAt.is_null())
        .filter(device::Column::Status.eq(device::DeviceStatus::Approved));
    if !family.is_empty() && !family.eq_ignore_ascii_case("ALL") {
        let pattern = format!("%{family}%");
        select = select.filter(Expr::col(device::Column::DeviceFamily).ilike(pattern));
    }
    if !state_filter.is_empty() && !state_filter.eq_ignore_ascii_case("all") {
        select = select.filter(Expr::cust_with_values(
            r#"state::text = $1"#,
            [state_filter],
        ));
    }
    if !search.is_empty() {
        let pattern = format!("%{search}%");
        select = select.filter(
            Condition::any()
                .add(Expr::col(device::Column::DeviceName).ilike(&pattern))
                .add(Expr::col(device::Column::SoftwareVersion).ilike(&pattern)),
        );
    }
    if !show_all {
        select = select.filter(Expr::cust(
            r#"(state::text IN ('free', 'busy') OR (
                "lastTestExecution" IS NOT NULL
                AND COALESCE("lastExecutionStatus"::text, '') NOT IN ('failed', 'completed', 'cancelled')
                AND state::text <> 'unknown'))"#,
        ));
    }
    select
}

#[async_trait]
impl UserDeviceRepository for PostgresDeviceRepository {
    async fn list_user_devices(
        &self,
        query: &UserDeviceListQuery,
    ) -> Result<DeviceList, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::user_devices",
            "list_user_devices",
        );
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(10).clamp(1, MAX_PAGE_SIZE);
        let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
        let search = query.search.as_deref().unwrap_or("").trim();
        let family = query.device_family.as_deref().unwrap_or("").trim();
        let state_filter = query.filter_by.as_deref().unwrap_or("").trim();
        let show_all = query.show_all.unwrap_or(false);
        let sort_by = match query.sort_by.as_deref() {
            Some(
                "updatedAt" | "deviceName" | "status" | "softwareVersion" | "lastTestExecution",
            ) => query.sort_by.as_deref().unwrap_or("createdAt"),
            _ => "createdAt",
        };
        let descending = query.desc.unwrap_or(true);

        let total_devices =
            user_device_filters(device::Entity::find(), family, state_filter, search, show_all)
                .count(self.orm())
                .await? as i64;

        let mut select =
            user_device_filters(device::Entity::find(), family, state_filter, search, show_all);
        select = match (sort_by, descending) {
            ("updatedAt" | "lastTestExecution", true) => {
                select.order_by_desc(device::Column::UpdatedAt)
            }
            ("updatedAt" | "lastTestExecution", false) => {
                select.order_by_asc(device::Column::UpdatedAt)
            }
            ("deviceName", true) => select.order_by_desc(device::Column::DeviceName),
            ("deviceName", false) => select.order_by_asc(device::Column::DeviceName),
            ("status", true) => select.order_by_desc(device::Column::Status),
            ("status", false) => select.order_by_asc(device::Column::Status),
            ("softwareVersion", true) => select.order_by_desc(device::Column::SoftwareVersion),
            ("softwareVersion", false) => select.order_by_asc(device::Column::SoftwareVersion),
            (_, true) => select.order_by_desc(device::Column::CreatedAt),
            (_, false) => select.order_by_asc(device::Column::CreatedAt),
        };
        let devices = select
            .limit(u64::from(limit))
            .offset(offset as u64)
            .all(self.orm())
            .await?;

        let mut data = Vec::with_capacity(devices.len());
        for device_model in devices {
            let interfaces = device_interface::Entity::find()
                .filter(device_interface::Column::DeviceId.eq(&device_model.device_id))
                .order_by_asc(device_interface::Column::Id)
                .all(self.orm())
                .await?
                .iter()
                .map(interface_json)
                .collect::<Vec<_>>();
            let executions = test_execution::Entity::find()
                .filter(test_execution::Column::DeviceId.eq(&device_model.device_id))
                .order_by_desc(test_execution::Column::UpdatedAt)
                .limit(1)
                .all(self.orm())
                .await?;
            let mut test_executions = Vec::new();
            for execution in executions {
                let cases = test_case::Entity::find()
                    .filter(test_case::Column::ExecutionId.eq(&execution.test_id))
                    .order_by_asc(test_case::Column::Id)
                    .all(self.orm())
                    .await?
                    .iter()
                    .map(test_case_json)
                    .collect::<Vec<_>>();
                let mut value = test_execution_json(&execution);
                value["testCases"] = serde_json::Value::Array(cases);
                test_executions.push(value);
            }
            let mut value = device_json(&device_model);
            value["interfaces"] = serde_json::Value::Array(interfaces);
            value["testExecutions"] = serde_json::Value::Array(test_executions);
            data.push(value);
        }

        Ok(DeviceList {
            data,
            total_pages: u64::try_from(total_devices)
                .unwrap_or(0)
                .div_ceil(u64::from(limit)),
            current_page: page,
            total_devices: u64::try_from(total_devices).unwrap_or(0),
            requested_count: 0,
            device_timers: BTreeMap::new(),
            device_timeouts: BTreeMap::new(),
            state_count: BTreeMap::new(),
        })
    }

    async fn active_devices(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::user_devices",
            "active_devices",
        );
        let devices = device::Entity::find()
            .filter(device::Column::DeletedAt.is_null())
            .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
            .order_by_desc(device::Column::CreatedAt)
            .all(self.orm())
            .await?;
        let mut data = Vec::with_capacity(devices.len());
        for device_model in devices {
            let interfaces = device_interface::Entity::find()
                .filter(device_interface::Column::DeviceId.eq(&device_model.device_id))
                .order_by_asc(device_interface::Column::Id)
                .all(self.orm())
                .await?
                .iter()
                .map(interface_json)
                .collect::<Vec<_>>();
            let mut value = device_json(&device_model);
            value["interfaces"] = serde_json::Value::Array(interfaces);
            data.push(value);
        }
        Ok(data)
    }

    async fn update_heartbeat_timeout(
        &self,
        device_id: &str,
        value: i64,
    ) -> Result<bool, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::user_devices",
            "update_heartbeat_timeout",
        );
        let timer = i32::try_from(value).unwrap_or(i32::MAX);
        let result = device::Entity::update_many()
            .col_expr(device::Column::HeartbeatTimer, Expr::value(timer))
            .col_expr(
                device::Column::UpdatedAt,
                Expr::value(chrono::Utc::now()),
            )
            .filter(device::Column::DeviceId.eq(device_id))
            .filter(device::Column::DeletedAt.is_null())
            .exec(self.orm())
            .await?;
        Ok(result.rows_affected > 0)
    }
}
