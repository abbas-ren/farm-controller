use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::devices::repository_types::ControllerHeartbeatResult;

use super::super::{
    ControllerHeartbeat, error::DeviceRepositoryError, validation::normalized_controller_id,
};

pub(crate) async fn record_controller(
    pool: &PgPool,
    heartbeat: &ControllerHeartbeat,
) -> Result<ControllerHeartbeatResult, DeviceRepositoryError> {
    let controller_id = normalized_controller_id(&heartbeat.uid)?;
    let timestamp = DateTime::<Utc>::from_timestamp(heartbeat.timestamp, 0).ok_or_else(|| {
        DeviceRepositoryError::Validation("heartbeat timestamp is out of range".to_owned())
    })?;
    let mut transaction = pool.begin().await?;
    let previous_state = sqlx::query_scalar::<_, String>(
        r#"SELECT state::text FROM device_controllers
           WHERE "deviceControllerId" = $1 AND "deletedAt" IS NULL FOR UPDATE"#,
    )
    .bind(&controller_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(previous_state) = previous_state else {
        transaction.commit().await?;
        return Ok(ControllerHeartbeatResult {
            state_changed: false,
            alert: None,
        });
    };
    sqlx::query(
        r#"
        UPDATE device_controllers
        SET state = 'active', "ipAddress" = $2, "updatedAt" = now()
        WHERE "deviceControllerId" = $1
        "#,
    )
    .bind(&controller_id)
    .bind(&heartbeat.ip)
    .execute(&mut *transaction)
    .await
    .map_err(|error| {
        tracing::error!(%error, controller_id, "failed to persist controller heartbeat state transition");
        error
    })?;

    sqlx::query(
        r#"
        INSERT INTO device_controller_heartbeats
            ("controllerId", timestamp, timeout, data)
        VALUES ($1, $2, 0, $3)
        "#,
    )
    .bind(&controller_id)
    .bind(timestamp)
    .bind(sqlx::types::Json(serde_json::to_value(heartbeat).map_err(
        |error| DeviceRepositoryError::Validation(error.to_string()),
    )?))
    .execute(&mut *transaction)
    .await?;

    let metrics_updated = sqlx::query(
        r#"
        UPDATE device_controller_metrics SET
            "cpuCurrent" = $2, "cpuTotal" = $3, "cpuUsagePercent" = $4,
            "memoryUsed" = $5, "memoryTotal" = $6, "memoryUsagePercent" = $7,
            "networkUpload" = $8, "networkDownload" = $9,
            "diskUsed" = $10, "diskTotal" = $11, "diskUsagePercent" = $12,
            "collectedAt" = $13, "updatedAt" = now()
        WHERE id = (
            SELECT id FROM device_controller_metrics
            WHERE "controllerId" = $1
            ORDER BY "updatedAt" DESC, id DESC LIMIT 1 FOR UPDATE
        )
        "#,
    )
    .bind(&controller_id)
    .bind(&heartbeat.cpu_current)
    .bind(&heartbeat.cpu_total)
    .bind(heartbeat.cpu_usage_percent)
    .bind(&heartbeat.memory_used)
    .bind(&heartbeat.memory_total)
    .bind(heartbeat.memory_usage_percent)
    .bind(&heartbeat.network_upload)
    .bind(&heartbeat.network_download)
    .bind(&heartbeat.disk_used)
    .bind(&heartbeat.disk_total)
    .bind(heartbeat.disk_usage_percent)
    .bind(timestamp)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if metrics_updated == 0 {
        sqlx::query(
            r#"
            INSERT INTO device_controller_metrics (
                "controllerId", "cpuCurrent", "cpuTotal", "cpuUsagePercent",
                "memoryUsed", "memoryTotal", "memoryUsagePercent",
                "networkUpload", "networkDownload", "diskUsed", "diskTotal",
                "diskUsagePercent", "collectedAt", "createdAt", "updatedAt"
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,now(),now())
            "#,
        )
        .bind(&controller_id)
        .bind(&heartbeat.cpu_current)
        .bind(&heartbeat.cpu_total)
        .bind(heartbeat.cpu_usage_percent)
        .bind(&heartbeat.memory_used)
        .bind(&heartbeat.memory_total)
        .bind(heartbeat.memory_usage_percent)
        .bind(&heartbeat.network_upload)
        .bind(&heartbeat.network_download)
        .bind(&heartbeat.disk_used)
        .bind(&heartbeat.disk_total)
        .bind(heartbeat.disk_usage_percent)
        .bind(timestamp)
        .execute(&mut *transaction)
        .await?;
    }
    let state_changed = previous_state != "active";
    let alert = if state_changed {
        Some(
            sqlx::query_scalar::<_, serde_json::Value>(
                r#"INSERT INTO alerts
                      (id, user_id, title, message, type, status, is_read,
                       device_controller_id, data, created_at, updated_at)
                   VALUES (gen_random_uuid(), 'ADMIN', 'Device Controller Online',
                           'Device controller ' || $1 || ' changed from not reachable to online',
                           'success', 'unread', false, $1,
                           jsonb_build_object('deviceControllerId', $1,
                                              'previousState', $2, 'state', 'active'),
                           now(), now())
                   RETURNING to_jsonb(alerts)"#,
            )
            .bind(&controller_id)
            .bind(&previous_state)
            .fetch_one(&mut *transaction)
            .await?,
        )
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
