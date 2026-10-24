use std::{sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    devices::{DEVICE_COMMAND_RETRY_DELAY, EDGE_CONTROLLER_PORT},
    persistence::entity::{device, fallback_update, log_entry, release, test_execution},
    state::AppState,
};

use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    TransactionTrait,
    sea_query::{Expr, LockBehavior, LockType},
};

#[derive(Debug)]
struct FallbackFlash {
    id: i32,
    device_id: String,
    test_id: String,
    fallback_path: String,
    ip_address: String,
    device_type: String,
    version: String,
}

pub(super) async fn worker(state: Arc<AppState>, cancellation: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.device_action_poll_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_cycle(&state).await,
        }
    }
    tracing::info!(worker = "fallback_flash", "worker stopped");
}

async fn run_cycle(state: &AppState) {
    let fallback = match claim_next(state).await {
        Ok(Some(fallback)) => fallback,
        Ok(None) => return,
        Err(error) => {
            tracing::error!(%error, worker = "fallback_flash", "failed to claim fallback flash");
            state.metrics.record_worker_run("fallback_flash", "failure");
            return;
        }
    };
    match dispatch(state, &fallback).await {
        Ok(()) => {
            state.metrics.record_worker_items("fallback_flash", 1);
            state.metrics.record_worker_run("fallback_flash", "success");
        }
        Err(error) => {
            tracing::error!(%error, fallback_id = fallback.id, worker = "fallback_flash", "fallback flash dispatch failed");
            if let Err(store_error) = fail(state, &fallback, &error).await {
                tracing::error!(%store_error, fallback_id = fallback.id, "failed to persist fallback dispatch failure");
            }
            state.metrics.record_worker_run("fallback_flash", "failure");
        }
    }
}

async fn claim_next(state: &AppState) -> Result<Option<FallbackFlash>, sea_orm::DbErr> {
    let database = state.database.as_ref().expect("worker requires database");
    let transaction = database.orm().begin().await?;
    let pending = fallback_update::Entity::find()
        .filter(fallback_update::Column::Status.eq(fallback_update::FallbackStatus::Pending))
        .filter(Expr::cust(
            r#"EXISTS (
                 SELECT 1 FROM devices device
                 WHERE device."deviceId" = "fallback_updates"."deviceId"
                   AND device.status::text = 'approved'
                   AND device."deletedAt" IS NULL
                   AND device."ipAddress" IS NOT NULL
                   AND device."deviceType" IS NOT NULL
               )"#,
        ))
        .filter(Expr::cust(
            r#"EXISTS (
                 SELECT 1 FROM releases release
                 WHERE release.id::text = "fallback_updates"."releaseId"::text
               )"#,
        ))
        .order_by_asc(fallback_update::Column::CreatedAt)
        .limit(1)
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .one(&transaction)
        .await?;
    let Some(pending) = pending else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let device = device::Entity::find()
        .filter(device::Column::DeviceId.eq(&pending.device_id))
        .one(&transaction)
        .await?
        .ok_or_else(|| sea_orm::DbErr::RecordNotFound("fallback device".into()))?;
    let release_id = Uuid::parse_str(&pending.release_id)
        .map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?;
    let release = release::Entity::find_by_id(release_id)
        .one(&transaction)
        .await?
        .ok_or_else(|| sea_orm::DbErr::RecordNotFound("fallback release".into()))?;
    let fallback = FallbackFlash {
        id: pending.id,
        device_id: pending.device_id.clone(),
        test_id: pending.test_id.clone(),
        fallback_path: pending.fallback_path.clone(),
        ip_address: device.ip_address.unwrap_or_default(),
        device_type: device.device_type.unwrap_or_default(),
        version: release.version,
    };
    let now = chrono::Utc::now();
    fallback_update::Entity::update_many()
        .col_expr(
            fallback_update::Column::Status,
            Expr::value(fallback_update::FallbackStatus::Flashing),
        )
        .col_expr(fallback_update::Column::UpdatedAt, Expr::value(now))
        .filter(fallback_update::Column::Id.eq(fallback.id))
        .exec(&transaction)
        .await?;
    test_execution::Entity::update_many()
        .col_expr(
            test_execution::Column::ExecutionPhase,
            Expr::value(Some(
                test_execution::ExecutionPhase::WaitForFallbackCompletion,
            )),
        )
        .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
        .filter(test_execution::Column::TestId.eq(&fallback.test_id))
        .exec(&transaction)
        .await?;
    transaction.commit().await?;
    Ok(Some(fallback))
}

async fn dispatch(state: &AppState, fallback: &FallbackFlash) -> Result<(), String> {
    let tftp_path = format!("{}/{}", fallback.device_type, fallback.version);
    let payload = flash_payload(fallback, &state.config.device.server_ip, &tftp_path);
    let url = reqwest::Url::parse(&format!(
        "http://{}:{EDGE_CONTROLLER_PORT}/flash",
        fallback.ip_address
    ))
    .map_err(|error| error.to_string())?;
    let client = crate::external_http::client(Duration::from_secs(
        state.config.device.request_timeout_seconds,
    ))
    .map_err(|error| error.to_string())?;
    let mut last_error = String::new();
    for _ in 0..crate::external_http::EXPLICIT_COMMAND_ATTEMPTS {
        match client.post(url.clone()).json(&payload).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => last_error = format!("device returned {}", response.status()),
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(DEVICE_COMMAND_RETRY_DELAY).await;
    }
    Err(last_error)
}

fn flash_payload(fallback: &FallbackFlash, server_ip: &str, tftp_path: &str) -> serde_json::Value {
    serde_json::json!({
        "nfs": fallback.fallback_path,
        "server_ip": server_ip,
        "image": format!("{tftp_path}/Image"),
        "dtb": format!("{tftp_path}/board.dtb"),
        "build_ver": fallback.version,
        "deviceId": fallback.device_id,
    })
}

async fn fail(
    state: &AppState,
    fallback: &FallbackFlash,
    error: &str,
) -> Result<(), sea_orm::DbErr> {
    let database = state.database.as_ref().expect("worker requires database");
    let transaction = database.orm().begin().await?;
    let now = chrono::Utc::now();
    fallback_update::Entity::update_many()
        .col_expr(
            fallback_update::Column::Status,
            Expr::value(fallback_update::FallbackStatus::Failed),
        )
        .col_expr(fallback_update::Column::UpdatedAt, Expr::value(now))
        .filter(fallback_update::Column::Id.eq(fallback.id))
        .exec(&transaction)
        .await?;
    test_execution::Entity::update_many()
        .col_expr(
            test_execution::Column::ExecutionPhase,
            Expr::value(Some(test_execution::ExecutionPhase::CompleteFailed)),
        )
        .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
        .filter(test_execution::Column::TestId.eq(&fallback.test_id))
        .exec(&transaction)
        .await?;
    log_entry::ActiveModel {
        r#type: Set(log_entry::LogEntryType::Test),
        reference_id: Set(fallback.test_id.clone()),
        data: Set(serde_json::json!({
            "message": format!("Fallback flash dispatch failed: {error}")
        })),
        level: Set(log_entry::LogEntryLevel::Error),
        timestamp: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FallbackFlash, flash_payload};

    #[test]
    fn payload_matches_direct_flash_contract() {
        let fallback = FallbackFlash {
            id: 1,
            device_id: "device-1".to_owned(),
            test_id: "42".to_owned(),
            fallback_path: "/nfs/gen4/1.2.3".to_owned(),
            ip_address: "192.0.2.5".to_owned(),
            device_type: "gen4".to_owned(),
            version: "1.2.3".to_owned(),
        };
        assert_eq!(
            flash_payload(&fallback, "192.0.2.10", "gen4/1.2.3"),
            serde_json::json!({
                "nfs": "/nfs/gen4/1.2.3",
                "server_ip": "192.0.2.10",
                "image": "gen4/1.2.3/Image",
                "dtb": "gen4/1.2.3/board.dtb",
                "build_ver": "1.2.3",
                "deviceId": "device-1",
            })
        );
    }
}
