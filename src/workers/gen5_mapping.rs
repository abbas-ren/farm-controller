use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{devices::EDGE_CONTROLLER_PORT, state::AppState};

const MAX_INITIAL_ATTEMPTS: i32 = 5;
const POLL_RETRY_SECONDS: i64 = 60;

#[derive(Debug, sqlx::FromRow)]
struct MappingJob {
    id: Uuid,
    mac_address: String,
    status: String,
    retry_count: i32,
    failure_dc_ip: Option<String>,
    baseline_mapping_count: Option<i32>,
    retry_schedule_index: Option<i32>,
    created_at: DateTime<Utc>,
}

pub(super) async fn worker(state: Arc<AppState>, cancellation: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.gen5_mapping_poll_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_cycle(&state).await,
        }
    }
    tracing::info!(worker = "gen5_mapping", "worker stopped");
}

async fn run_cycle(state: &AppState) {
    for _ in 0..state.config.workers.gen5_mapping_batch_size {
        let job = match claim_next(state).await {
            Ok(Some(job)) => job,
            Ok(None) => break,
            Err(error) => {
                record_failure(state, &error.to_string(), "failed to claim mapping job");
                break;
            }
        };
        let result = if job.status == "queued" {
            process_queued(state, &job).await
        } else {
            process_failure_poll(state, &job).await
        };
        match result {
            Ok(processed) => {
                state.metrics.record_worker_run("gen5_mapping", "success");
                state
                    .metrics
                    .record_worker_items("gen5_mapping", u64::from(processed));
            }
            Err(error) => record_failure(state, &error, "mapping job failed"),
        }
    }
}

async fn claim_next(state: &AppState) -> Result<Option<MappingJob>, sqlx::Error> {
    let database = state.database.as_ref().expect("worker requires database");
    let mut transaction = database.pool().begin().await?;
    let job = sqlx::query_as::<_, MappingJob>(
        r#"SELECT id, "macAddress" AS mac_address, status::text AS status,
                  "retryCount" AS retry_count, "failureDcIp" AS failure_dc_ip,
                  "baselineMappingCount" AS baseline_mapping_count,
                  "retryScheduleIndex" AS retry_schedule_index,
                  "createdAt" AS created_at
           FROM gen5_mapping_queue
           WHERE status::text = 'queued'
              OR (status::text = 'failure_polling' AND "nextRetryAt" <= now())
           ORDER BY CASE WHEN status::text = 'queued' THEN 0 ELSE 1 END,
                    COALESCE("nextRetryAt", "createdAt") ASC
           LIMIT 1 FOR UPDATE SKIP LOCKED"#,
    )
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(job) = job else {
        transaction.rollback().await?;
        return Ok(None);
    };
    if job.status == "queued" {
        sqlx::query(
            r#"UPDATE gen5_mapping_queue
               SET status = 'running', "lastError" = NULL, "updatedAt" = now()
               WHERE id = $1 AND status::text = 'queued'"#,
        )
        .bind(job.id)
        .execute(&mut *transaction)
        .await?;
    } else {
        let lease_seconds =
            i64::try_from(state.config.workers.gen5_mapping_request_timeout_seconds)
                .unwrap_or(i64::MAX)
                + 5;
        sqlx::query(
            r#"UPDATE gen5_mapping_queue
               SET "nextRetryAt" = now() + ($2 * interval '1 second'), "updatedAt" = now()
               WHERE id = $1 AND status::text = 'failure_polling'"#,
        )
        .bind(job.id)
        .bind(lease_seconds)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Some(job))
}

async fn process_queued(state: &AppState, job: &MappingJob) -> Result<u32, String> {
    if job.mac_address.trim().is_empty() {
        update_initial_failure(state, job, "macAddress is missing", true).await?;
        return Ok(1);
    }
    let database = state.database.as_ref().expect("worker requires database");
    let controllers = sqlx::query_scalar::<_, String>(
        r#"SELECT "ipAddress" FROM device_controllers
           WHERE status::text = 'approved' AND state::text = 'active'
             AND "deletedAt" IS NULL
           ORDER BY "updatedAt" DESC"#,
    )
    .fetch_all(database.pool())
    .await
    .map_err(|error| error.to_string())?;
    if controllers.is_empty() {
        update_initial_failure(
            state,
            job,
            "No active approved device controllers available",
            false,
        )
        .await?;
        return Ok(1);
    }
    for controller in controllers {
        if let Err(error) = trigger_mapping(state, &controller, &job.mac_address).await {
            tracing::warn!(%error, controller_ip = controller, job_id = %job.id, "Gen5 mapping trigger failed");
        }
    }
    sqlx::query(
        r#"UPDATE gen5_mapping_queue
           SET status = 'waiting', "lastError" = NULL, "updatedAt" = now()
           WHERE id = $1 AND status::text = 'running'"#,
    )
    .bind(job.id)
    .execute(database.pool())
    .await
    .map_err(|error| error.to_string())?;
    Ok(1)
}

async fn update_initial_failure(
    state: &AppState,
    job: &MappingJob,
    message: &str,
    force_cancel: bool,
) -> Result<(), String> {
    let database = state.database.as_ref().expect("worker requires database");
    let attempts = job.retry_count.saturating_add(1);
    let status = if force_cancel || attempts >= MAX_INITIAL_ATTEMPTS {
        "cancelled"
    } else {
        "queued"
    };
    sqlx::query(
        r#"UPDATE gen5_mapping_queue
           SET status = $2::"enum_gen5_mapping_queue_status", "retryCount" = $3,
               "lastError" = $4, "updatedAt" = now()
           WHERE id = $1 AND status::text = 'running'"#,
    )
    .bind(job.id)
    .bind(status)
    .bind(attempts)
    .bind(message)
    .execute(database.pool())
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn process_failure_poll(state: &AppState, job: &MappingJob) -> Result<u32, String> {
    let max_age = chrono::Duration::minutes(
        i64::try_from(state.config.workers.gen5_mapping_max_window_minutes).unwrap_or(i64::MAX),
    );
    if Utc::now().signed_duration_since(job.created_at) >= max_age {
        cancel_and_delete_device(state, job, "Exceeded mapping retry window").await?;
        return Ok(1);
    }
    let Some(controller_ip) = job.failure_dc_ip.as_deref() else {
        cancel_and_delete_device(state, job, "Missing device controller IP").await?;
        return Ok(1);
    };
    let count = match mapping_entry_count(state, controller_ip).await {
        Ok(count) => count,
        Err(error) => {
            schedule_poll(state, job, Some(&error)).await?;
            return Ok(1);
        }
    };
    let baseline = job.baseline_mapping_count.unwrap_or(0);
    if count > baseline {
        if let Err(error) = trigger_mapping(state, controller_ip, &job.mac_address).await {
            tracing::warn!(%error, %controller_ip, job_id = %job.id, "Gen5 mapping retrigger failed");
        }
        let database = state.database.as_ref().expect("worker requires database");
        sqlx::query(
            r#"UPDATE gen5_mapping_queue
               SET status = 'waiting', "lastError" = NULL,
                   "baselineMappingCount" = $2, "nextRetryAt" = NULL,
                   "retryScheduleIndex" = NULL, "updatedAt" = now()
               WHERE id = $1 AND status::text = 'failure_polling'"#,
        )
        .bind(job.id)
        .bind(count)
        .execute(database.pool())
        .await
        .map_err(|error| error.to_string())?;
    } else {
        schedule_poll(state, job, None).await?;
    }
    Ok(1)
}

async fn schedule_poll(
    state: &AppState,
    job: &MappingJob,
    error: Option<&str>,
) -> Result<(), String> {
    let database = state.database.as_ref().expect("worker requires database");
    let next_index = job.retry_schedule_index.unwrap_or(0).saturating_add(1);
    let max_polls =
        i32::try_from(state.config.workers.gen5_mapping_max_window_minutes).unwrap_or(i32::MAX);
    if next_index >= max_polls {
        return cancel_and_delete_device(state, job, error.unwrap_or("Mapping retries exhausted"))
            .await;
    }
    sqlx::query(
        r#"UPDATE gen5_mapping_queue
           SET "retryScheduleIndex" = $2,
               "nextRetryAt" = now() + ($3 * interval '1 second'),
               "lastError" = COALESCE($4, "lastError"), "updatedAt" = now()
           WHERE id = $1 AND status::text = 'failure_polling'"#,
    )
    .bind(job.id)
    .bind(next_index)
    .bind(POLL_RETRY_SECONDS)
    .bind(error)
    .execute(database.pool())
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn cancel_and_delete_device(
    state: &AppState,
    job: &MappingJob,
    message: &str,
) -> Result<(), String> {
    let database = state.database.as_ref().expect("worker requires database");
    let mut transaction = database
        .pool()
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    let updated = sqlx::query(
        r#"UPDATE gen5_mapping_queue
           SET status = 'cancelled', "lastError" = $2, "nextRetryAt" = NULL,
               "updatedAt" = now()
           WHERE id = $1 AND status::text = 'failure_polling'"#,
    )
    .bind(job.id)
    .bind(message)
    .execute(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?
    .rows_affected();
    if updated > 0 {
        sqlx::query(
            r#"DELETE FROM devices
               WHERE regexp_replace(lower("macAddress"), '[:-]', '', 'g') =
                     regexp_replace(lower($1), '[:-]', '', 'g')"#,
        )
        .bind(&job.mac_address)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
    }
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn trigger_mapping(state: &AppState, controller_ip: &str, mac: &str) -> Result<(), String> {
    let url = controller_url(controller_ip, "gen5/tty_entry")?;
    let client = mapping_client(state, 1)?;
    client
        .post(url)
        .json(&serde_json::json!({"mac": mac.to_ascii_lowercase()}))
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn mapping_entry_count(state: &AppState, controller_ip: &str) -> Result<i32, String> {
    let url = controller_url(controller_ip, "mapping/entry")?;
    let client = mapping_client(
        state,
        state.config.workers.gen5_mapping_request_timeout_seconds,
    )?;
    let payload: serde_json::Value = client
        .post(url)
        .json(&serde_json::json!({"gen": 5}))
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())?;
    let count = payload
        .get("uart")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    i32::try_from(count).map_err(|_| "mapping entry count exceeds i32".to_owned())
}

fn controller_url(controller_ip: &str, path: &str) -> Result<reqwest::Url, String> {
    reqwest::Url::parse(&format!(
        "http://{controller_ip}:{EDGE_CONTROLLER_PORT}/{path}"
    ))
    .map_err(|error| error.to_string())
}

fn mapping_client(state: &AppState, timeout_seconds: u64) -> Result<reqwest::Client, String> {
    let _ = state;
    crate::external_http::client(Duration::from_secs(timeout_seconds))
        .map_err(|error| error.to_string())
}

fn record_failure(state: &AppState, error: &str, message: &str) {
    state.metrics.record_worker_run("gen5_mapping", "failure");
    tracing::error!(%error, worker = "gen5_mapping", "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_routes_are_scoped_to_the_edgecontroller_port() {
        assert_eq!(
            controller_url("192.0.2.1", "mapping/entry")
                .unwrap()
                .as_str(),
            "http://192.0.2.1:8888/mapping/entry"
        );
    }

    #[test]
    fn poll_schedule_is_one_minute_with_a_configured_hard_limit() {
        assert_eq!(POLL_RETRY_SECONDS, 60);
        assert_eq!(MAX_INITIAL_ATTEMPTS, 5);
    }
}
