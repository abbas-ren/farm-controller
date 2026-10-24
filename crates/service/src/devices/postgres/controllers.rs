use super::*;
use super::json_models::{
    controller_json, device_json, relay_channel_json, relay_json,
};

use crate::persistence::{
    entity::{
        device, device_controller, device_interface, device_state_change, log_entry, relay,
        relay_channel,
    },
    orm_util,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, LockType, extension::postgres::PgExpr},
};

fn controller_list_filters(
    mut select: sea_orm::Select<device_controller::Entity>,
    status: &str,
    controller_state: &str,
    search: &str,
) -> sea_orm::Select<device_controller::Entity> {
    select = select.filter(device_controller::Column::DeletedAt.is_null());
    if !status.is_empty() && !status.eq_ignore_ascii_case("all") {
        select = select.filter(
            Expr::cust_with_values(
                r#"status::text ILIKE ('%' || $1 || '%')"#,
                [status],
            ),
        );
    }
    if !controller_state.is_empty() && !controller_state.eq_ignore_ascii_case("all") {
        select = select.filter(Expr::cust_with_values(
            r#"state::text ILIKE ('%' || $1 || '%')"#,
            [controller_state],
        ));
    }
    if !search.is_empty() {
        let pattern = format!("%{search}%");
        select = select.filter(
            Condition::any()
                .add(
                    Expr::col(device_controller::Column::DeviceControllerId).ilike(&pattern),
                )
                .add(Expr::col(device_controller::Column::Name).ilike(&pattern))
                .add(Expr::col(device_controller::Column::MacAddress).ilike(&pattern))
                .add(Expr::col(device_controller::Column::IpAddress).ilike(&pattern)),
        );
    }
    select
}

#[async_trait]
impl ControllerRepository for PostgresDeviceRepository {
    async fn device_action_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceActionTarget>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::controllers",
            "device_action_target",
        );
        Ok(device::Entity::find()
            .filter(device::Column::DeviceId.eq(device_id))
            .filter(device::Column::DeletedAt.is_null())
            .filter(device::Column::IpAddress.is_not_null())
            .one(self.orm())
            .await?
            .and_then(|model| {
                let ip_address = model.ip_address?;
                Some(DeviceActionTarget {
                    device_id: model.device_id,
                    ip_address,
                    status: json_models::device_status_str(model.status.as_ref())
                        .unwrap_or("requested")
                        .to_owned(),
                })
            }))
    }

    async fn apply_device_action(
        &self,
        device_id: &str,
        action: DeviceAction,
        user_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::controllers",
            "apply_device_action",
        );
        let transaction = self.orm().begin().await?;
        let device = device::Entity::find()
            .filter(device::Column::DeviceId.eq(device_id))
            .filter(device::Column::DeletedAt.is_null())
            .lock(LockType::Update)
            .one(&transaction)
            .await?;
        let Some(device_model) = device else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let mut device_value = device_json(&device_model);
        match action {
            DeviceAction::Declined => {
                let now = chrono::Utc::now();
                log_entry::ActiveModel {
                    r#type: Set(log_entry::LogEntryType::Device),
                    reference_id: Set(device_id.to_owned()),
                    data: Set(serde_json::json!({
                        "action": "declined",
                        "deviceData": device_value,
                        "timestamp": now,
                    })),
                    level: Set(log_entry::LogEntryLevel::Info),
                    timestamp: Set(now),
                    created_at: Set(now),
                    updated_at: Set(now),
                    ..Default::default()
                }
                .insert(&transaction)
                .await?;
                device_interface::Entity::delete_many()
                    .filter(device_interface::Column::DeviceId.eq(device_id))
                    .exec(&transaction)
                    .await?;
                device::Entity::delete_many()
                    .filter(device::Column::DeviceId.eq(device_id))
                    .exec(&transaction)
                    .await?;
            }
            DeviceAction::Approved => {
                let now = chrono::Utc::now();
                let updated = device::Entity::update_many()
                    .col_expr(
                        device::Column::Status,
                        Expr::value(device::DeviceStatus::Approved),
                    )
                    .col_expr(device::Column::State, Expr::value(device::DeviceState::Free))
                    .col_expr(device::Column::UpdatedBy, Expr::value(user_id.to_owned()))
                    .col_expr(device::Column::UpdatedAt, Expr::value(now))
                    .col_expr(device::Column::StateUpdatedAt, Expr::value(now))
                    .filter(device::Column::DeviceId.eq(device_id))
                    .filter(device::Column::Status.eq(device::DeviceStatus::Requested))
                    .filter(device::Column::DeletedAt.is_null())
                    .exec(&transaction)
                    .await?;
                if updated.rows_affected == 0 {
                    return Err(DeviceRepositoryError::Validation(
                        "Device is no longer awaiting approval".to_owned(),
                    ));
                }
                let refreshed = device::Entity::find()
                    .filter(device::Column::DeviceId.eq(device_id))
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| {
                        DeviceRepositoryError::Internal(
                            "approved device disappeared".to_owned(),
                        )
                    })?;
                device_value = device_json(&refreshed);
                device_state_change::ActiveModel {
                    device_id: Set(device_id.to_owned()),
                    state: Set(device_state_change::DeviceStateChangeState::Free),
                    changed_at: Set(now),
                    created_at: Set(now),
                    updated_at: Set(now),
                    ..Default::default()
                }
                .insert(&transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        Ok(Some(device_value))
    }

    async fn list_controllers(
        &self,
        query: &ControllerListQuery,
    ) -> Result<ControllerList, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::controllers",
            "list_controllers",
        );
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(20).clamp(1, MAX_PAGE_SIZE);
        let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
        let search = query.search.as_deref().unwrap_or("").trim();
        let status = query.status.as_deref().unwrap_or("").trim();
        let controller_state = query.state.as_deref().unwrap_or("").trim();
        let sort_by = match query.sort_by.as_deref() {
            Some(
                "updatedAt" | "name" | "deviceFamily" | "macAddress" | "ipAddress" | "status"
                | "state",
            ) => query.sort_by.as_deref().unwrap_or("createdAt"),
            _ => "createdAt",
        };
        let descending = query.desc.unwrap_or(true);

        let filtered = controller_list_filters(
            device_controller::Entity::find(),
            status,
            controller_state,
            search,
        );
        let total_count = filtered.clone().count(self.orm()).await? as i64;

        let mut select = controller_list_filters(
            device_controller::Entity::find(),
            status,
            controller_state,
            search,
        );
        select = match (sort_by, descending) {
            ("updatedAt", true) => select.order_by_desc(device_controller::Column::UpdatedAt),
            ("updatedAt", false) => select.order_by_asc(device_controller::Column::UpdatedAt),
            ("name", true) => select.order_by_desc(device_controller::Column::Name),
            ("name", false) => select.order_by_asc(device_controller::Column::Name),
            ("deviceFamily", true) => {
                select.order_by_desc(device_controller::Column::DeviceFamily)
            }
            ("deviceFamily", false) => {
                select.order_by_asc(device_controller::Column::DeviceFamily)
            }
            ("macAddress", true) => select.order_by_desc(device_controller::Column::MacAddress),
            ("macAddress", false) => select.order_by_asc(device_controller::Column::MacAddress),
            ("ipAddress", true) => select.order_by_desc(device_controller::Column::IpAddress),
            ("ipAddress", false) => select.order_by_asc(device_controller::Column::IpAddress),
            ("status", true) => select.order_by_desc(device_controller::Column::Status),
            ("status", false) => select.order_by_asc(device_controller::Column::Status),
            ("state", true) => select.order_by_desc(device_controller::Column::State),
            ("state", false) => select.order_by_asc(device_controller::Column::State),
            (_, true) => select.order_by_desc(device_controller::Column::CreatedAt),
            (_, false) => select.order_by_asc(device_controller::Column::CreatedAt),
        };
        let controllers = select
            .limit(u64::from(limit))
            .offset(offset as u64)
            .all(self.orm())
            .await?;

        let mut data = Vec::with_capacity(controllers.len());
        for controller in controllers {
            let relays = relay::Entity::find()
                .filter(
                    relay::Column::DeviceControllerId.eq(&controller.device_controller_id),
                )
                .filter(relay::Column::DeletedAt.is_null())
                .order_by_asc(relay::Column::CreatedAt)
                .all(self.orm())
                .await?;
            let mut relay_values = Vec::with_capacity(relays.len());
            for relay_model in relays {
                let channels = relay_channel::Entity::find()
                    .filter(relay_channel::Column::RelayId.eq(relay_model.id))
                    .filter(relay_channel::Column::DeletedAt.is_null())
                    .order_by_asc(relay_channel::Column::ChannelNumber)
                    .all(self.orm())
                    .await?;
                let mut channel_values = Vec::with_capacity(channels.len());
                for channel in channels {
                    let mut channel_value = relay_channel_json(&channel);
                    let device_value = if let Some(device_id) = channel.device_id.as_deref() {
                        device::Entity::find()
                            .filter(device::Column::DeviceId.eq(device_id))
                            .one(self.orm())
                            .await?
                            .map(|d| {
                                serde_json::json!({
                                    "deviceId": d.device_id,
                                    "macAddress": d.mac_address,
                                    "deviceName": d.device_name,
                                    "deviceType": d.device_type,
                                    "ipAddress": d.ip_address,
                                })
                            })
                    } else {
                        None
                    };
                    channel_value["device"] = device_value
                        .unwrap_or(serde_json::Value::Null);
                    channel_values.push(channel_value);
                }
                let mut relay_value = relay_json(&relay_model);
                relay_value["relayChannels"] = serde_json::Value::Array(channel_values);
                relay_values.push(relay_value);
            }
            let mut controller_value = controller_json(&controller);
            controller_value["relays"] = serde_json::Value::Array(relay_values);
            data.push(controller_value);
        }

        let all_controllers = device_controller::Entity::find()
            .filter(device_controller::Column::DeletedAt.is_null())
            .all(self.orm())
            .await?;
        let controller_total = all_controllers.len() as i64;
        let controller_active = all_controllers
            .iter()
            .filter(|c| matches!(c.state, device_controller::DeviceControllerState::Active))
            .count() as i64;
        let controller_not_reachable = all_controllers
            .iter()
            .filter(|c| {
                matches!(
                    c.state,
                    device_controller::DeviceControllerState::NotReachable
                )
            })
            .count() as i64;

        let all_relays = relay::Entity::find()
            .filter(relay::Column::DeletedAt.is_null())
            .all(self.orm())
            .await?;
        let relay_total = all_relays.len() as i64;
        let relay_connected = all_relays
            .iter()
            .filter(|r| matches!(r.state, relay::RelayState::Connected))
            .count() as i64;

        Ok(ControllerList {
            data,
            total_pages: u64::try_from(total_count)
                .unwrap_or(0)
                .div_ceil(u64::from(limit)),
            current_page: page,
            total_count: u64::try_from(total_count).unwrap_or(0),
            summary: serde_json::json!({
                "controllers": {
                    "total": controller_total,
                    "active": controller_active,
                    "notReachable": controller_not_reachable
                },
                "relays": {
                    "total": relay_total,
                    "connected": relay_connected,
                    "disconnected": (relay_total - relay_connected).max(0)
                }
            }),
        })
    }

    async fn edit_controller(
        &self,
        controller_id: &str,
        request: &ControllerEditRequest,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::controllers",
            "edit_controller",
        );
        controller_store::edit(self.orm(), controller_id, request).await
    }

    async fn controller_delete_target(
        &self,
        controller_id: &str,
    ) -> Result<Option<ControllerDeleteTarget>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::controllers",
            "controller_delete_target",
        );
        controller_store::delete_target(self.orm(), controller_id).await
    }

    async fn delete_controller(&self, controller_id: &str) -> Result<bool, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::controllers",
            "delete_controller",
        );
        controller_store::delete(self.orm(), controller_id).await
    }

    async fn device_delete_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceDeleteTarget>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::controllers",
            "device_delete_target",
        );
        #[derive(Debug, sea_orm::FromQueryResult)]
        struct DeleteTargetRow {
            ip_address: String,
            device_family: Option<String>,
            controller_id: Option<String>,
            controller_ip: Option<String>,
            relay_serial: Option<String>,
            relay_channel: Option<i32>,
        }
        Ok(orm_util::query_one::<_, DeleteTargetRow>(
            self.orm(),
            r#"SELECT COALESCE(d."ipAddress", '') AS ip_address,
                 d."deviceFamily" AS device_family,
                 COALESCE(r."deviceControllerId", d."controllerId") AS controller_id,
                 dc."ipAddress" AS controller_ip, r."serialNumber" AS relay_serial,
                 rc."channelNumber" AS relay_channel
               FROM devices d
               LEFT JOIN relay_channels rc ON rc."deviceId" = d."deviceId" AND rc."deletedAt" IS NULL
               LEFT JOIN relays r ON r.id = rc."relayId" AND r."deletedAt" IS NULL
               LEFT JOIN device_controllers dc ON dc."deviceControllerId" = COALESCE(r."deviceControllerId", d."controllerId")
               WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL
               ORDER BY r."updatedAt" DESC NULLS LAST LIMIT 1"#,
            vec![device_id.into()],
        )
        .await?
        .map(|row| DeviceDeleteTarget {
            ip_address: row.ip_address,
            device_family: row.device_family,
            controller_id: row.controller_id,
            controller_ip: row.controller_ip,
            relay_serial: row.relay_serial,
            relay_channel: row.relay_channel,
        }))
    }

    async fn delete_device(&self, device_id: &str) -> Result<bool, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::controllers",
            "delete_device",
        );
        let transaction = self.orm().begin().await?;
        orm_util::execute(
            &transaction,
            r#"UPDATE device_controllers controller
               SET mappings = COALESCE((
                       SELECT jsonb_object_agg(entry.key, entry.value)
                       FROM jsonb_each(COALESCE(controller.mappings, '{}'::jsonb)) entry
                       WHERE regexp_replace(lower(entry.key), '[:-]', '', 'g') <> $1
                   ), '{}'::jsonb),
                   "updatedAt" = now()
               WHERE EXISTS (
                   SELECT 1
                   FROM jsonb_each(COALESCE(controller.mappings, '{}'::jsonb)) entry
                   WHERE regexp_replace(lower(entry.key), '[:-]', '', 'g') = $1
               )"#,
            vec![device_id.into()],
        )
        .await?;
        device_interface::Entity::delete_many()
            .filter(device_interface::Column::DeviceId.eq(device_id))
            .exec(&transaction)
            .await?;
        relay_channel::Entity::delete_many()
            .filter(relay_channel::Column::DeviceId.eq(device_id))
            .exec(&transaction)
            .await?;
        let deleted = device::Entity::delete_many()
            .filter(device::Column::DeviceId.eq(device_id))
            .exec(&transaction)
            .await?
            .rows_affected
            > 0;
        transaction.commit().await?;
        Ok(deleted)
    }
}
