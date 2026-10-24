use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection,
    EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect, Set, Statement,
    TransactionTrait, sea_query::Expr,
};
use uuid::Uuid;

use crate::{
    devices::repository_types::DeviceHeartbeatResult,
    persistence::entity::{alert, device, device_state_change, heartbeat},
};

use super::super::error::DeviceRepositoryError;
use super::{interface, state_transition::process_device_post_update};

pub(crate) async fn record_device(
    orm: &DatabaseConnection,
    device_id: &str,
    heartbeat_payload: &serde_json::Value,
) -> Result<Option<DeviceHeartbeatResult>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::heartbeat_store::device", "record_device");
    if !heartbeat_payload.is_object() {
        return Err(DeviceRepositoryError::Validation(
            "device heartbeat must be a JSON object".to_owned(),
        ));
    }
    let heartbeat_data = interface::interface_heartbeat_data(heartbeat_payload);
    let interface_statuses = interface::parse_interface_statuses(&heartbeat_data);
    let transaction = orm.begin().await?;

    // Complex FOR UPDATE + EXISTS active-work check — keep as raw SQL.
    #[derive(Debug, FromQueryResult)]
    struct LockedDevice {
        state: String,
        has_active_work: bool,
    }
    let device_row = LockedDevice::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT COALESCE(device.state::text, 'unknown') AS state,
                  (EXISTS (SELECT 1 FROM device_action_queue action
                           WHERE action."targetDeviceId" = device."deviceId"
                             AND action.status::text IN ('queued', 'waiting', 'running'))
                   OR EXISTS (SELECT 1 FROM fallback_updates fallback
                              WHERE fallback."deviceId" = device."deviceId"
                                AND fallback.status::text IN ('pending', 'flashing')))
                  AS has_active_work
           FROM devices device
           WHERE device."deviceId" = $1
             AND device."deletedAt" IS NULL
             AND device.status::text = 'approved'
           FOR UPDATE OF device"#,
        [device_id.into()],
    ))
    .one(&transaction)
    .await?;
    let Some(LockedDevice {
        state: previous_state,
        has_active_work,
    }) = device_row
    else {
        transaction.rollback().await?;
        return Ok(None);
    };

    let existing = heartbeat::Entity::find()
        .filter(heartbeat::Column::DeviceId.eq(device_id))
        .order_by_desc(heartbeat::Column::Timestamp)
        .lock(sea_orm::sea_query::LockType::Update)
        .one(&transaction)
        .await?;
    let now = chrono::Utc::now();
    match existing {
        Some(row) if row.data == *heartbeat_payload => {
            heartbeat::Entity::update_many()
                .col_expr(heartbeat::Column::Timestamp, Expr::value(now))
                .col_expr(heartbeat::Column::Timeout, Expr::value(0))
                .filter(heartbeat::Column::Id.eq(row.id))
                .exec(&transaction)
                .await?;
        }
        _ => {
            heartbeat::ActiveModel {
                device_id: Set(device_id.to_owned()),
                timestamp: Set(now),
                timeout: Set(0),
                data: Set(heartbeat_payload.clone()),
                ..Default::default()
            }
            .insert(&transaction)
            .await?;
        }
    }

    let state = if has_active_work {
        device::DeviceState::Busy
    } else {
        device::DeviceState::Free
    };
    let state_text = if has_active_work { "busy" } else { "free" };
    device::Entity::update_many()
        .col_expr(device::Column::State, Expr::value(Some(state)))
        .col_expr(device::Column::StateUpdatedAt, Expr::value(Some(now)))
        .col_expr(device::Column::LastConnectedOn, Expr::value(now))
        .col_expr(device::Column::Upgrading, Expr::value(Some(false)))
        .col_expr(device::Column::Flashing, Expr::value(Some(false)))
        .col_expr(device::Column::UpdatedAt, Expr::value(now))
        .filter(device::Column::DeviceId.eq(device_id))
        .exec(&transaction)
        .await
        .map_err(|error| {
            tracing::error!(%error, device_id, target_state = state_text, "failed to persist device heartbeat state transition");
            error
        })?;

    let change_state = if has_active_work {
        device_state_change::DeviceStateChangeState::Busy
    } else {
        device_state_change::DeviceStateChangeState::Free
    };
    device_state_change::ActiveModel {
        device_id: Set(device_id.to_owned()),
        state: Set(change_state),
        changed_at: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&transaction)
    .await?;

    let mut alerts = Vec::new();
    if !has_active_work && matches!(previous_state.as_str(), "faulty" | "not_reachable") {
        let inserted = alert::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set("ADMIN".to_owned()),
            title: Set("Device comes as Available".to_owned()),
            message: Set(format!("Device {device_id} is now available")),
            r#type: Set(alert::AlertType::Success),
            status: Set(alert::AlertStatus::Unread),
            is_read: Set(false),
            device_id: Set(Some(device_id.to_owned())),
            data: Set(Some(serde_json::json!({
                "deviceId": device_id,
                "previousState": previous_state,
                "state": "free",
            }))),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&transaction)
        .await?;
        alerts.push(alert_json(&inserted));
    }

    // Interface rows still use raw SQL (legacy `type` column vs entity `interfaceType`).
    let mut interface_changes = Vec::new();
    for interface_status in interface_statuses {
        if let Some(interface_id) = interface_status.interface_id.as_deref() {
            #[derive(Debug, FromQueryResult)]
            struct ExistingInterface {
                id: i32,
                status: String,
            }
            let existing = ExistingInterface::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT id, status::text AS status FROM device_interfaces
                   WHERE "deviceId" = $1 AND type = $2 AND "interfaceId" = $3
                   FOR UPDATE"#,
                [
                    device_id.into(),
                    interface_status.interface_type.clone().into(),
                    interface_id.into(),
                ],
            ))
            .one(&transaction)
            .await?;
            match existing {
                Some(row) if row.status != interface_status.status => {
                    interface::update_interface_status(
                        &transaction,
                        row.id,
                        interface_status.status,
                    )
                    .await?;
                    interface_changes.push(interface::InterfaceChange {
                        interface_type: interface_status.interface_type,
                        interface_id: interface_id.to_owned(),
                        status: interface_status.status,
                        previous_status: Some(row.status),
                    });
                }
                None => {
                    transaction
                        .execute_raw(Statement::from_sql_and_values(
                            DatabaseBackend::Postgres,
                            r#"INSERT INTO device_interfaces
                                  ("deviceId", "interfaceId", type, status, "createdAt", "updatedAt")
                               VALUES ($1, $2, $3, ($4::text)::"enum_device_interfaces_status",
                                       now(), now())"#,
                            [
                                device_id.into(),
                                interface_id.into(),
                                interface_status.interface_type.clone().into(),
                                interface_status.status.into(),
                            ],
                        ))
                        .await?;
                    interface_changes.push(interface::InterfaceChange {
                        interface_type: interface_status.interface_type,
                        interface_id: interface_id.to_owned(),
                        status: interface_status.status,
                        previous_status: None,
                    });
                }
                Some(_) => {}
            }
        } else {
            #[derive(Debug, FromQueryResult)]
            struct ExistingInterface {
                id: i32,
                interface_id: String,
                status: String,
            }
            let existing = ExistingInterface::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT id, "interfaceId" AS interface_id, status::text AS status
                   FROM device_interfaces
                   WHERE "deviceId" = $1 AND type = $2 FOR UPDATE"#,
                [
                    device_id.into(),
                    interface_status.interface_type.clone().into(),
                ],
            ))
            .all(&transaction)
            .await?;
            for row in existing {
                if row.status == interface_status.status {
                    continue;
                }
                interface::update_interface_status(&transaction, row.id, interface_status.status)
                    .await?;
                interface_changes.push(interface::InterfaceChange {
                    interface_type: interface_status.interface_type.clone(),
                    interface_id: row.interface_id,
                    status: interface_status.status,
                    previous_status: Some(row.status),
                });
            }
        }
    }

    for change in &interface_changes {
        if interface::is_healthy_interface_status(change.status) {
            continue;
        }
        let interface_label = format!("{}:{}", change.interface_type, change.interface_id);
        let inserted = alert::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set("ADMIN".to_owned()),
            title: Set(format!(
                "Interface {} {}",
                interface_label, change.status
            )),
            message: Set(format!(
                "Device {device_id} interface {interface_label} status is {}",
                change.status
            )),
            r#type: Set(alert::AlertType::Warning),
            status: Set(alert::AlertStatus::Unread),
            is_read: Set(false),
            device_id: Set(Some(device_id.to_owned())),
            data: Set(Some(serde_json::json!({
                "alertCategory": "interface-status",
                "deviceId": device_id,
                "interfaceType": change.interface_type,
                "interfaceId": change.interface_id,
                "interfaceStatus": change.status,
                "previousInterfaceStatus": change.previous_status,
            }))),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&transaction)
        .await?;
        alerts.push(alert_json(&inserted));
    }

    let (post_update_events, deferred_cancellations, deferred_test_logs) =
        process_device_post_update(
            &transaction,
            device_id,
            heartbeat_payload
                .get("nfsPath")
                .and_then(serde_json::Value::as_str),
        )
        .await?;
    transaction.commit().await?;
    let state_changed = previous_state != state_text;
    if matches!(previous_state.as_str(), "faulty" | "not_reachable") && state_text == "free" {
        tracing::info!(
            device_id,
            previous_state,
            "device heartbeat recovered to free"
        );
    }
    Ok(Some(DeviceHeartbeatResult {
        state: state_text.to_owned(),
        state_changed,
        interface_changes: interface_changes
            .into_iter()
            .map(interface::interface_change_value)
            .collect(),
        heartbeat_data,
        alerts,
        post_update_events,
        deferred_cancellations,
        deferred_test_logs,
    }))
}

fn alert_json(model: &alert::Model) -> serde_json::Value {
    let _api_trace = crate::observability::ApiTrace::enter("devices::heartbeat_store::device", "alert_json");
    serde_json::json!({
        "id": model.id,
        "user_id": model.user_id,
        "title": model.title,
        "message": model.message,
        "type": match model.r#type {
            alert::AlertType::Info => "info",
            alert::AlertType::Success => "success",
            alert::AlertType::Warning => "warning",
            alert::AlertType::Error => "error",
        },
        "status": match model.status {
            alert::AlertStatus::Unread => "unread",
            alert::AlertStatus::Read => "read",
        },
        "is_read": model.is_read,
        "device_id": model.device_id,
        "data": model.data,
        "created_at": model.created_at,
        "updated_at": model.updated_at,
    })
}

