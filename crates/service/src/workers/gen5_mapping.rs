//! Gen5 MAC→controller mapping queue (legacy `Gen5MappingQueueService`).
//!
//! Claims queued and failure-poll jobs, triggers mapping on the DC, retries on a
//! 1-minute cadence within a 30-minute window, then deletes the device if mapping
//! never resolves.

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    persistence::entity::{device, device_controller, gen5_mapping_queue},
    state::AppState,
};

use sea_orm::{ColumnTrait, EntityTrait, ExprTrait, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
    sea_query::{Expr, LockBehavior, LockType},
};

const MAX_INITIAL_ATTEMPTS: i32 = 5;
const POLL_RETRY_SECONDS: i64 = 60;

#[derive(Debug)]
struct MappingJob {
    id: Uuid,
    mac_address: String,
    status: gen5_mapping_queue::QueueJobStatus,
    retry_count: i32,
    failure_dc_ip: Option<String>,
    baseline_mapping_count: Option<i32>,
    retry_schedule_index: Option<i32>,
    created_at: DateTime<Utc>,
}

impl From<gen5_mapping_queue::Model> for MappingJob {
    /// Build a `MappingJob` view from a SeaORM queue model.
    fn from(model: gen5_mapping_queue::Model) -> Self {
        Self {
            id: model.id,
            mac_address: model.mac_address,
            status: model
                .status
                .unwrap_or(gen5_mapping_queue::QueueJobStatus::Queued),
            retry_count: model.retry_count,
            failure_dc_ip: model.failure_dc_ip,
            baseline_mapping_count: model.baseline_mapping_count,
            retry_schedule_index: model.retry_schedule_index,
            created_at: model.created_at,
        }
    }
}

/// Poll loop for Gen5 mapping queue + failure-poll retries.
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

/// Claim one job (queued or due poll) and process it.
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
        let result = if job.status == gen5_mapping_queue::QueueJobStatus::Queued {
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

/// SKIP LOCKED claim: prefer QUEUED, else FAILURE_POLLING with `next_retry_at` <= now.
async fn claim_next(state: &AppState) -> Result<Option<MappingJob>, sea_orm::DbErr> {
    let database = state.database.as_ref().expect("worker requires database");
    let transaction = database.orm().begin().await?;
    let now = Utc::now();
    let model = gen5_mapping_queue::Entity::find()
        .filter(
            sea_orm::Condition::any()
                .add(
                    gen5_mapping_queue::Column::Status
                        .eq(gen5_mapping_queue::QueueJobStatus::Queued),
                )
                .add(
                    sea_orm::Condition::all()
                        .add(
                            gen5_mapping_queue::Column::Status
                                .eq(gen5_mapping_queue::QueueJobStatus::FailurePolling),
                        )
                        .add(gen5_mapping_queue::Column::NextRetryAt.lte(now)),
                ),
        )
        .order_by_asc::<Expr>(
            Expr::case(
                // Cast enum literal — plain text compare fails with enum_…_status = text.
                Expr::col(gen5_mapping_queue::Column::Status).eq(
                    crate::persistence::pg_enum::pg_enum(
                        &gen5_mapping_queue::QueueJobStatus::Queued,
                    ),
                ),
                0,
            )
            .finally(1)
            .into(),
        )
        .order_by_asc::<Expr>(
            Expr::cust("COALESCE(\"nextRetryAt\", \"createdAt\")").into(),
        )
        .limit(1)
        // SKIP LOCKED: concurrent worker ticks must not claim the same row.
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .one(&transaction)
        .await?;
    let Some(model) = model else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let job = MappingJob::from(model);
    if job.status == gen5_mapping_queue::QueueJobStatus::Queued {
        gen5_mapping_queue::Entity::update_many()
            .col_expr(
                gen5_mapping_queue::Column::Status,
                crate::persistence::pg_enum::pg_enum(&gen5_mapping_queue::QueueJobStatus::Running),
            )
            .col_expr(
                gen5_mapping_queue::Column::LastError,
                Expr::value(Option::<String>::None),
            )
            .col_expr(gen5_mapping_queue::Column::UpdatedAt, Expr::value(now))
            .filter(gen5_mapping_queue::Column::Id.eq(job.id))
            .filter(
                gen5_mapping_queue::Column::Status.eq(gen5_mapping_queue::QueueJobStatus::Queued),
            )
            .exec(&transaction)
            .await?;
    } else {
        let lease_seconds =
            i64::try_from(state.config.workers.gen5_mapping_request_timeout_seconds)
                .unwrap_or(i64::MAX)
                + 5;
        gen5_mapping_queue::Entity::update_many()
            .col_expr(
                gen5_mapping_queue::Column::NextRetryAt,
                Expr::cust_with_values(
                    "now() + ($1 * interval '1 second')",
                    [lease_seconds],
                ),
            )
            .col_expr(gen5_mapping_queue::Column::UpdatedAt, Expr::value(now))
            .filter(gen5_mapping_queue::Column::Id.eq(job.id))
            .filter(
                gen5_mapping_queue::Column::Status
                    .eq(gen5_mapping_queue::QueueJobStatus::FailurePolling),
            )
            .exec(&transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(Some(job))
}

/// Trigger mapping on the DC for a freshly queued MAC; start failure-poll on error.
async fn process_queued(state: &AppState, job: &MappingJob) -> Result<u32, String> {
    if job.mac_address.trim().is_empty() {
        update_initial_failure(state, job, "macAddress is missing", true).await?;
        return Ok(1);
    }
    let database = state.database.as_ref().expect("worker requires database");
    let controllers = device_controller::Entity::find()
        .filter(
            device_controller::Column::Status.eq(device_controller::DeviceControllerStatus::Approved),
        )
        .filter(
            device_controller::Column::State.eq(device_controller::DeviceControllerState::Active),
        )
        .filter(device_controller::Column::DeletedAt.is_null())
        .order_by_desc(device_controller::Column::UpdatedAt)
        .select_only()
        .column(device_controller::Column::IpAddress)
        .into_tuple::<String>()
        .all(database.orm())
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
    let now = Utc::now();
    gen5_mapping_queue::Entity::update_many()
        .col_expr(
            gen5_mapping_queue::Column::Status,
            crate::persistence::pg_enum::pg_enum(&gen5_mapping_queue::QueueJobStatus::Waiting),
        )
        .col_expr(
            gen5_mapping_queue::Column::LastError,
            Expr::value(Option::<String>::None),
        )
        .col_expr(gen5_mapping_queue::Column::UpdatedAt, Expr::value(now))
        .filter(gen5_mapping_queue::Column::Id.eq(job.id))
        .filter(gen5_mapping_queue::Column::Status.eq(gen5_mapping_queue::QueueJobStatus::Running))
        .exec(database.orm())
        .await
        .map_err(|error| error.to_string())?;
    Ok(1)
}

/// Move a job into FAILURE_POLLING with the first retry timestamp.
async fn update_initial_failure(
    state: &AppState,
    job: &MappingJob,
    message: &str,
    force_cancel: bool,
) -> Result<(), String> {
    let database = state.database.as_ref().expect("worker requires database");
    let attempts = job.retry_count.saturating_add(1);
    let status = if force_cancel || attempts >= MAX_INITIAL_ATTEMPTS {
        gen5_mapping_queue::QueueJobStatus::Cancelled
    } else {
        gen5_mapping_queue::QueueJobStatus::Queued
    };
    let now = Utc::now();
    gen5_mapping_queue::Entity::update_many()
        .col_expr(gen5_mapping_queue::Column::Status, crate::persistence::pg_enum::pg_enum(&status))
        .col_expr(gen5_mapping_queue::Column::RetryCount, Expr::value(attempts))
        .col_expr(
            gen5_mapping_queue::Column::LastError,
            Expr::value(Some(message.to_owned())),
        )
        .col_expr(gen5_mapping_queue::Column::UpdatedAt, Expr::value(now))
        .filter(gen5_mapping_queue::Column::Id.eq(job.id))
        .filter(gen5_mapping_queue::Column::Status.eq(gen5_mapping_queue::QueueJobStatus::Running))
        .exec(database.orm())
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Re-check mapping; schedule next 1-min retry or delete device after 30-min window.
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
        let now = Utc::now();
        gen5_mapping_queue::Entity::update_many()
            .col_expr(
                gen5_mapping_queue::Column::Status,
                crate::persistence::pg_enum::pg_enum(&gen5_mapping_queue::QueueJobStatus::Waiting),
            )
            .col_expr(
                gen5_mapping_queue::Column::LastError,
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                gen5_mapping_queue::Column::BaselineMappingCount,
                Expr::value(Some(count)),
            )
            .col_expr(
                gen5_mapping_queue::Column::NextRetryAt,
                Expr::value(Option::<DateTime<Utc>>::None),
            )
            .col_expr(
                gen5_mapping_queue::Column::RetryScheduleIndex,
                Expr::value(Option::<i32>::None),
            )
            .col_expr(gen5_mapping_queue::Column::UpdatedAt, Expr::value(now))
            .filter(gen5_mapping_queue::Column::Id.eq(job.id))
            .filter(
                gen5_mapping_queue::Column::Status
                    .eq(gen5_mapping_queue::QueueJobStatus::FailurePolling),
            )
            .exec(database.orm())
            .await
            .map_err(|error| error.to_string())?;
    } else {
        schedule_poll(state, job, None).await?;
    }
    Ok(1)
}

/// Bump retry count and set `next_retry_at` = now + 1 minute.
async fn schedule_poll(
    state: &AppState,
    job: &MappingJob,
    error: Option<&str>,
) -> Result<(), String> {
    let database = state.database.as_ref().expect("worker requires database");
    // Retry cadence: one poll per minute; `max_window_minutes` is the hard limit (legacy 30).
    let next_index = job.retry_schedule_index.unwrap_or(0).saturating_add(1);
    let max_polls =
        i32::try_from(state.config.workers.gen5_mapping_max_window_minutes).unwrap_or(i32::MAX);
    if next_index >= max_polls {
        return cancel_and_delete_device(state, job, error.unwrap_or("Mapping retries exhausted"))
            .await;
    }
    let now = Utc::now();
    let mut update = gen5_mapping_queue::Entity::update_many()
        .col_expr(
            gen5_mapping_queue::Column::RetryScheduleIndex,
            Expr::value(Some(next_index)),
        )
        .col_expr(
            gen5_mapping_queue::Column::NextRetryAt,
            Expr::cust_with_values(
                "now() + ($1 * interval '1 second')",
                [POLL_RETRY_SECONDS],
            ),
        )
        .col_expr(gen5_mapping_queue::Column::UpdatedAt, Expr::value(now))
        .filter(gen5_mapping_queue::Column::Id.eq(job.id))
        .filter(
            gen5_mapping_queue::Column::Status
                .eq(gen5_mapping_queue::QueueJobStatus::FailurePolling),
        );
    if let Some(error) = error {
        update = update.col_expr(
            gen5_mapping_queue::Column::LastError,
            Expr::value(Some(error.to_owned())),
        );
    }
    update
        .exec(database.orm())
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Cancel the mapping job and delete the unmapped device after max window.
async fn cancel_and_delete_device(
    state: &AppState,
    job: &MappingJob,
    message: &str,
) -> Result<(), String> {
    let database = state.database.as_ref().expect("worker requires database");
    let transaction = database
        .orm()
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    let now = Utc::now();
    let updated = gen5_mapping_queue::Entity::update_many()
        .col_expr(
            gen5_mapping_queue::Column::Status,
            crate::persistence::pg_enum::pg_enum(&gen5_mapping_queue::QueueJobStatus::Cancelled),
        )
        .col_expr(
            gen5_mapping_queue::Column::LastError,
            Expr::value(Some(message.to_owned())),
        )
        .col_expr(
            gen5_mapping_queue::Column::NextRetryAt,
            Expr::value(Option::<DateTime<Utc>>::None),
        )
        .col_expr(gen5_mapping_queue::Column::UpdatedAt, Expr::value(now))
        .filter(gen5_mapping_queue::Column::Id.eq(job.id))
        .filter(
            gen5_mapping_queue::Column::Status
                .eq(gen5_mapping_queue::QueueJobStatus::FailurePolling),
        )
        .exec(&transaction)
        .await
        .map_err(|error| error.to_string())?
        .rows_affected;
    if updated > 0 {
        device::Entity::delete_many()
            .filter(Expr::cust_with_values(
                r#"regexp_replace(lower("macAddress"), '[:-]', '', 'g') =
                   regexp_replace(lower($1), '[:-]', '', 'g')"#,
                [job.mac_address.clone()],
            ))
            .exec(&transaction)
            .await
            .map_err(|error| error.to_string())?;
    }
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// HTTP call to the device controller to start MAC mapping.
async fn trigger_mapping(state: &AppState, controller_ip: &str, mac: &str) -> Result<(), String> {
    let url = controller_url(
        controller_ip,
        "gen5/tty_entry",
        state.config.device.edge_controller_http_port,
    )?;
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

/// Read how many mapping entries the controller currently reports.
async fn mapping_entry_count(state: &AppState, controller_ip: &str) -> Result<i32, String> {
    let url = controller_url(
        controller_ip,
        "mapping/entry",
        state.config.device.edge_controller_http_port,
    )?;
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

/// Build an absolute URL for a controller HTTP path.
fn controller_url(controller_ip: &str, path: &str, port: u16) -> Result<reqwest::Url, String> {
    reqwest::Url::parse(&format!("http://{controller_ip}:{port}/{path}"))
        .map_err(|error| error.to_string())
}

/// reqwest client with the mapping timeout from config.
fn mapping_client(state: &AppState, timeout_seconds: u64) -> Result<reqwest::Client, String> {
    let _ = state;
    crate::external_http::client(Duration::from_secs(timeout_seconds))
        .map_err(|error| error.to_string())
}

/// Record Gen5 mapping worker failure metrics.
fn record_failure(state: &AppState, error: &str, message: &str) {
    state.metrics.record_worker_run("gen5_mapping", "failure");
    tracing::error!(%error, worker = "gen5_mapping", "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Assert mapping HTTP URLs target the configured edge-controller port.
    fn controller_routes_are_scoped_to_the_edgecontroller_port() {
        assert_eq!(
            controller_url("192.0.2.1", "mapping/entry", 8888)
                .unwrap()
                .as_str(),
            "http://192.0.2.1:8888/mapping/entry"
        );
    }

    #[test]
    /// Assert retry interval is 1 minute within the 30-minute hard window.
    fn poll_schedule_is_one_minute_with_a_configured_hard_limit() {
        assert_eq!(POLL_RETRY_SECONDS, 60);
        assert_eq!(MAX_INITIAL_ATTEMPTS, 5);
    }
}
