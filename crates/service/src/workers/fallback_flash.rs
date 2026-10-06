use std::{sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;

use crate::{
    devices::{DEVICE_COMMAND_RETRY_DELAY, EDGE_CONTROLLER_PORT},
    state::AppState,
};

#[derive(Debug, sqlx::FromRow)]
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

async fn claim_next(state: &AppState) -> Result<Option<FallbackFlash>, sqlx::Error> {
    let database = state.database.as_ref().expect("worker requires database");
    let mut transaction = database.pool().begin().await?;
    let fallback = sqlx::query_as::<_, FallbackFlash>(
        r#"SELECT fallback.id, fallback."deviceId" AS device_id,
                  fallback."testId" AS test_id, fallback."fallbackPath" AS fallback_path,
                  device."ipAddress" AS ip_address, device."deviceType" AS device_type,
                  release.version
           FROM fallback_updates fallback
           JOIN devices device ON device."deviceId" = fallback."deviceId"
           JOIN releases release ON release.id::text = fallback."releaseId"::text
           WHERE fallback.status::text = 'pending'
             AND device.status::text = 'approved' AND device."deletedAt" IS NULL
                         AND device."ipAddress" IS NOT NULL AND device."deviceType" IS NOT NULL
           ORDER BY fallback."createdAt" ASC
           LIMIT 1 FOR UPDATE OF fallback SKIP LOCKED"#,
    )
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(fallback) = fallback else {
        transaction.rollback().await?;
        return Ok(None);
    };
    sqlx::query(
        r#"UPDATE fallback_updates SET status = 'flashing', "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(fallback.id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"UPDATE test_executions
           SET "executionPhase" = 'WAIT_FOR_FALLBACK_COMPLETION', "updatedAt" = now()
           WHERE "testId" = $1"#,
    )
    .bind(&fallback.test_id)
    .execute(&mut *transaction)
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

async fn fail(state: &AppState, fallback: &FallbackFlash, error: &str) -> Result<(), sqlx::Error> {
    let database = state.database.as_ref().expect("worker requires database");
    let mut transaction = database.pool().begin().await?;
    sqlx::query(
        r#"UPDATE fallback_updates SET status = 'failed', "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(fallback.id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"UPDATE test_executions
           SET "executionPhase" = 'COMPLETE_FAILED', "updatedAt" = now()
           WHERE "testId" = $1"#,
    )
    .bind(&fallback.test_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"INSERT INTO log_entries
              (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
           VALUES ('test', $1, jsonb_build_object('message', $2::text), 'error',
                   now(), now(), now())"#,
    )
    .bind(&fallback.test_id)
    .bind(format!("Fallback flash dispatch failed: {error}"))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await
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
