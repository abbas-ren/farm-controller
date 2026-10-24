//! Record controller heartbeats and emit reachability alerts.
//!
//! Ports Node `ControllerHeartbeatQueueService` persistence.

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
    sea_query::LockType,
};
use uuid::Uuid;

use crate::{
    devices::repository_types::ControllerHeartbeatResult,
    persistence::entity::{
        alert, device_controller, device_controller_heartbeat, device_controller_metrics,
    },
};

use super::super::{
    ControllerHeartbeat, error::DeviceRepositoryError, validation::normalized_controller_id,
};

/// PURPOSE: Persist controller heartbeat and emit reachability alerts on change.
/// USE: Controller heartbeat worker (Node ControllerHeartbeatQueueService).
pub(crate) async fn record_controller(
    orm: &DatabaseConnection,
    heartbeat: &ControllerHeartbeat,
) -> Result<ControllerHeartbeatResult, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::heartbeat_store::controller",
        "record_controller",
    );
    let controller_id = normalized_controller_id(&heartbeat.uid)?;
    let timestamp = DateTime::<Utc>::from_timestamp(heartbeat.timestamp, 0).ok_or_else(|| {
        DeviceRepositoryError::Validation("heartbeat timestamp is out of range".to_owned())
    })?;
    let transaction = orm.begin().await?;
    let controller = device_controller::Entity::find()
        .filter(device_controller::Column::DeviceControllerId.eq(&controller_id))
        .filter(device_controller::Column::DeletedAt.is_null())
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
        .one(&transaction)
        .await?;
    let Some(controller) = controller else {
        transaction.commit().await?;
        return Ok(ControllerHeartbeatResult {
            state_changed: false,
            alert: None,
        });
    };
    let previous_state = match controller.state {
        device_controller::DeviceControllerState::Active => "active",
        device_controller::DeviceControllerState::NotReachable => "not-reachable",
    };
    let now = Utc::now();
    device_controller::ActiveModel {
        device_controller_id: Set(controller.device_controller_id.clone()),
        state: Set(device_controller::DeviceControllerState::Active),
        ip_address: Set(heartbeat.ip.clone()),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(&transaction)
    .await
    .map_err(|error| {
        tracing::error!(%error, controller_id, "failed to persist controller heartbeat state transition");
        error
    })?;

    device_controller_heartbeat::ActiveModel {
        controller_id: Set(controller_id.clone()),
        timestamp: Set(timestamp),
        timeout: Set(0),
        data: Set(serde_json::to_value(heartbeat).map_err(|error| {
            DeviceRepositoryError::Validation(error.to_string())
        })?),
        ..Default::default()
    }
    .insert(&transaction)
    .await?;

    let latest_metrics = device_controller_metrics::Entity::find()
        .filter(device_controller_metrics::Column::ControllerId.eq(&controller_id))
        .order_by_desc(device_controller_metrics::Column::UpdatedAt)
        .order_by_desc(device_controller_metrics::Column::Id)
        .limit(1)
        // Row lock: serialize concurrent updates on this controller/device/relay.
        .lock(LockType::Update)
        .one(&transaction)
        .await?;
    if let Some(metrics) = latest_metrics {
        device_controller_metrics::ActiveModel {
            id: Set(metrics.id),
            cpu_current: Set(heartbeat.cpu_current.clone()),
            cpu_total: Set(heartbeat.cpu_total.clone()),
            cpu_usage_percent: Set(heartbeat.cpu_usage_percent),
            memory_used: Set(heartbeat.memory_used.clone()),
            memory_total: Set(heartbeat.memory_total.clone()),
            memory_usage_percent: Set(heartbeat.memory_usage_percent),
            network_upload: Set(heartbeat.network_upload.clone()),
            network_download: Set(heartbeat.network_download.clone()),
            disk_used: Set(heartbeat.disk_used.clone()),
            disk_total: Set(heartbeat.disk_total.clone()),
            disk_usage_percent: Set(heartbeat.disk_usage_percent),
            collected_at: Set(timestamp),
            updated_at: Set(now),
            ..Default::default()
        }
        .update(&transaction)
        .await?;
    } else {
        device_controller_metrics::ActiveModel {
            controller_id: Set(controller_id.clone()),
            cpu_current: Set(heartbeat.cpu_current.clone()),
            cpu_total: Set(heartbeat.cpu_total.clone()),
            cpu_usage_percent: Set(heartbeat.cpu_usage_percent),
            memory_used: Set(heartbeat.memory_used.clone()),
            memory_total: Set(heartbeat.memory_total.clone()),
            memory_usage_percent: Set(heartbeat.memory_usage_percent),
            network_upload: Set(heartbeat.network_upload.clone()),
            network_download: Set(heartbeat.network_download.clone()),
            disk_used: Set(heartbeat.disk_used.clone()),
            disk_total: Set(heartbeat.disk_total.clone()),
            disk_usage_percent: Set(heartbeat.disk_usage_percent),
            collected_at: Set(timestamp),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&transaction)
        .await?;
    }
    let state_changed = previous_state != "active";
    let alert = if state_changed {
        let inserted = alert::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set("ADMIN".to_owned()),
            title: Set("Device Controller Online".to_owned()),
            message: Set(format!(
                "Device controller {controller_id} changed from not reachable to online"
            )),
            r#type: Set(alert::AlertType::Success),
            status: Set(alert::AlertStatus::Unread),
            is_read: Set(false),
            device_controller_id: Set(Some(controller_id.clone())),
            data: Set(Some(serde_json::json!({
                "deviceControllerId": controller_id,
                "previousState": previous_state,
                "state": "active",
            }))),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&transaction)
        .await?;
        Some(controller_alert_json(&inserted))
    } else {
        None
    };
    transaction.commit().await?;
    if state_changed {
        tracing::info!(
            controller_id,
            previous_state,
            "controller heartbeat recovered to active"
        );
    }
    Ok(ControllerHeartbeatResult {
        state_changed,
        alert,
    })
}

/// PURPOSE: Serialize `controller alert json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
fn controller_alert_json(model: &alert::Model) -> serde_json::Value {
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
        "device_controller_id": model.device_controller_id,
        "data": model.data,
        "created_at": model.created_at,
        "updated_at": model.updated_at,
    })
}
