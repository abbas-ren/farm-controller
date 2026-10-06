use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{events::ServerEvent, state::AppState, test_catalog::TestRunCreation};

use super::artifact_preparation::{artifact_paths, prepare_artifacts, safe_segment};

#[derive(Debug, sqlx::FromRow)]
struct Preparation {
    test_id: String,
    device_type: String,
    build_id: String,
    build_version: String,
    test_plan_id: i64,
    test_plan_name: String,
    name: String,
    is_all_selected: bool,
    created_by: String,
    test_cycle_id: Option<String>,
    artifacts: serde_json::Value,
}

#[derive(Debug, sqlx::FromRow)]
struct PreparationCase {
    test_case_id: i64,
    script_file: String,
}

pub(super) async fn worker(state: Arc<AppState>, cancellation: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.test_preparation_poll_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_cycle(&state).await,
        }
    }
    tracing::info!(worker = "test_preparation", "worker stopped");
}

async fn run_cycle(state: &AppState) {
    for _ in 0..state.config.workers.test_preparation_batch_size {
        let preparation = match claim(state).await {
            Ok(Some(preparation)) => preparation,
            Ok(None) => break,
            Err(error) => {
                record_failure(state, &error);
                break;
            }
        };
        let test_id = preparation.test_id.clone();
        match prepare(state, preparation).await {
            Ok(event) => {
                state.metrics.record_worker_items("test_preparation", 1);
                state
                    .metrics
                    .record_worker_run("test_preparation", "success");
                if let Err(error) = state.event_publisher.publish(event) {
                    tracing::warn!(%error, %test_id, "failed to publish prepared test");
                }
            }
            Err(error) => {
                tracing::error!(%error, %test_id, worker = "test_preparation", "test preparation failed");
                if let Some(event) = mark_failed(state, &test_id, &error).await {
                    let _ = state.event_publisher.publish(event);
                }
                state
                    .metrics
                    .record_worker_run("test_preparation", "failure");
            }
        }
    }
}

async fn claim(state: &AppState) -> Result<Option<Preparation>, sqlx::Error> {
    let database = state.database.as_ref().expect("worker requires database");
    let lease_seconds =
        i64::try_from(state.config.workers.test_preparation_lease_seconds).unwrap_or(i64::MAX);
    sqlx::query_as::<_, Preparation>(
        r#"WITH candidate AS (
               SELECT execution."testId"
               FROM test_executions execution
               WHERE execution.status::text = 'not_executed'
                 AND (execution."executionPhase" = 'PREPARE_ARTIFACTS'
                      OR (execution."executionPhase" = 'PREPARE_TEST_SCRIPTS'
                          AND execution."updatedAt" < now() - ($1 * interval '1 second')))
                 AND COALESCE(execution."cancelRequested", false) = false
               ORDER BY execution."createdAt" ASC
               LIMIT 1
               FOR UPDATE SKIP LOCKED
           ), claimed AS (
               UPDATE test_executions execution
               SET "executionPhase" = 'PREPARE_TEST_SCRIPTS', "updatedAt" = now()
               FROM candidate
               WHERE execution."testId" = candidate."testId"
               RETURNING execution.*
           )
           SELECT claimed."testId" AS test_id, claimed."deviceType" AS device_type,
                claimed."buildId"::text AS build_id,
                COALESCE(claimed."buildVersion", release.version) AS build_version,
                  claimed."testPlanId"::bigint AS test_plan_id,
                COALESCE(claimed."testPlanName", '') AS test_plan_name,
                COALESCE(claimed.name, claimed."testId") AS name,
                COALESCE(claimed."isAllSelected", false) AS is_all_selected,
                COALESCE(claimed."createdBy", 'system') AS created_by,
                  claimed."testCycleId" AS test_cycle_id, release.artifacts
           FROM claimed
           JOIN releases release ON release.id::text = claimed."buildId"::text"#,
    )
    .bind(lease_seconds)
    .fetch_optional(database.pool())
    .await
}

async fn prepare(state: &AppState, mut preparation: Preparation) -> Result<ServerEvent, String> {
    let database = state.database.as_ref().expect("worker requires database");
    let cases = sqlx::query_as::<_, PreparationCase>(
        r#"SELECT "testCaseId"::bigint AS test_case_id,
              COALESCE("scriptFile", '') AS script_file
           FROM testcase WHERE "executionId" = $1 ORDER BY "suiteId", id"#,
    )
    .bind(&preparation.test_id)
    .fetch_all(database.pool())
    .await
    .map_err(|error| error.to_string())?;
    let paths = artifact_paths(&preparation.artifacts)?;
    let nfs_root = PathBuf::from(&state.config.device.nfs_host_path);
    let tftp_root = PathBuf::from(&state.config.device.tftp_output_dir);
    let device_type = safe_segment(&preparation.device_type)?;
    let test_id = safe_segment(&preparation.test_id)?;
    let nfs_path = nfs_root.join(device_type).join(test_id);
    let tftp_path = tftp_root.join(device_type).join(test_id);
    let nfs_for_task = nfs_path.clone();
    let tftp_for_task = tftp_path.clone();
    tokio::task::spawn_blocking(move || prepare_artifacts(&paths, &nfs_for_task, &tftp_for_task))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;

    let catalog = state
        .test_catalog
        .as_ref()
        .ok_or_else(|| "TestRail is not configured".to_owned())?;
    if preparation.test_cycle_id.is_none() {
        let case_ids = cases
            .iter()
            .map(|case| u64::try_from(case.test_case_id).unwrap_or_default())
            .collect();
        let run_id = catalog
            .create_run(&TestRunCreation {
                suite_id: u64::try_from(preparation.test_plan_id).unwrap_or_default(),
                name: preparation.name.clone(),
                description: format!(
                    "Automated {} test run for build {}",
                    preparation.test_plan_name, preparation.build_version
                ),
                include_all: preparation.is_all_selected,
                case_ids,
            })
            .await
            .map_err(|error| error.to_string())?;
        let run_id = run_id.to_string();
        sqlx::query(
            r#"UPDATE test_executions SET "testCycleId" = $2, "updatedAt" = now()
               WHERE "testId" = $1"#,
        )
        .bind(&preparation.test_id)
        .bind(&run_id)
        .execute(database.pool())
        .await
        .map_err(|error| error.to_string())?;
        preparation.test_cycle_id = Some(run_id);
    }

    let scripts_dir = nfs_path
        .join("usr")
        .join("src")
        .join("tests")
        .join(&preparation.test_id);
    tokio::fs::create_dir_all(&scripts_dir)
        .await
        .map_err(|error| error.to_string())?;
    for case in cases {
        let script_path = case.script_file.trim().trim_start_matches('/');
        if script_path.is_empty() {
            return Err(format!(
                "missing script for test case {}",
                case.test_case_id
            ));
        }
        let bytes = catalog
            .test_script(script_path)
            .await
            .map_err(|error| error.to_string())?;
        let script = String::from_utf8(bytes)
            .map_err(|_| format!("script for test case {} is not UTF-8", case.test_case_id))?
            .replace("\r\n", "\n");
        let destination = scripts_dir.join(format!("{}.sh", case.test_case_id));
        tokio::fs::write(&destination, script)
            .await
            .map_err(|error| error.to_string())?;
        set_executable(&destination)
            .await
            .map_err(|error| error.to_string())?;
    }

    let queue_id = Uuid::new_v4();
    let mut transaction = database
        .pool()
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query(
        r#"INSERT INTO device_action_queue
              (id, "testId", "deviceType", "userId", type, status, mode,
               "releaseId", "createdAt", "updatedAt")
           SELECT $1, $2, $3, $4, 'test', 'queued', 'deviceType', $5::uuid, now(), now()
           WHERE NOT EXISTS (SELECT 1 FROM device_action_queue WHERE "testId" = $2)"#,
    )
    .bind(queue_id)
    .bind(&preparation.test_id)
    .bind(&preparation.device_type)
    .bind(&preparation.created_by)
    .bind(&preparation.build_id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?;
    let persisted_queue_id = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM device_action_queue WHERE "testId" = $1
           ORDER BY "createdAt" ASC LIMIT 1"#,
    )
    .bind(&preparation.test_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?;
    sqlx::query(
        r#"UPDATE test_executions
           SET "executionPhase" = 'QUEUE_TEST_EXECUTION', "queueId" = $2,
               "updatedAt" = now()
           WHERE "testId" = $1 AND status::text = 'not_executed'
             AND COALESCE("cancelRequested", false) = false"#,
    )
    .bind(&preparation.test_id)
    .bind(persisted_queue_id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?;
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())?;
    Ok(prepared_event(
        &preparation.test_id,
        &preparation.created_by,
    ))
}

#[cfg(unix)]
async fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = tokio::fs::metadata(path).await?.permissions();
    permissions.set_mode(0o755);
    tokio::fs::set_permissions(path, permissions).await
}

#[cfg(not(unix))]
async fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

async fn mark_failed(state: &AppState, test_id: &str, error: &str) -> Option<ServerEvent> {
    let database = state.database.as_ref()?;
    let owner = sqlx::query_as::<_, (String, String)>(
        r#"UPDATE test_executions
           SET status = 'failed', "executionPhase" = 'COMPLETE_FAILED',
               "endedAt" = now(), "updatedAt" = now()
           WHERE "testId" = $1
           RETURNING COALESCE("createdBy", 'system'), "buildId"::text"#,
    )
    .bind(test_id)
    .fetch_optional(database.pool())
    .await
    .ok()??;
    Some(ServerEvent {
        event: "test_execution".to_owned(),
        payload: serde_json::json!({
            "testId": test_id,
            "type": "status",
            "data": {"status": "failed", "message": error, "timestamp": chrono::Utc::now().to_rfc3339()},
        }),
        room: Some(format!("user:{}", owner.0)),
    })
}

fn prepared_event(test_id: &str, user_id: &str) -> ServerEvent {
    ServerEvent {
        event: "test_execution".to_owned(),
        payload: serde_json::json!({
            "testId": test_id,
            "type": "queued",
            "data": {"status": "not_executed", "timestamp": chrono::Utc::now().to_rfc3339()},
        }),
        room: Some(format!("user:{user_id}")),
    }
}

fn record_failure(state: &AppState, error: &sqlx::Error) {
    state
        .metrics
        .record_worker_run("test_preparation", "failure");
    tracing::error!(%error, worker = "test_preparation", "failed to claim test preparation");
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

    use super::*;

    #[test]
    fn prepares_rootfs_and_boot_artifacts_in_scoped_paths() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("Image");
        let dtb = directory.path().join("board.dtb");
        let rootfs = directory.path().join("root.tar.bz2");
        std::fs::write(&image, b"image").unwrap();
        std::fs::write(&dtb, b"dtb").unwrap();
        let encoder = bzip2::write::BzEncoder::new(
            File::create(&rootfs).unwrap(),
            bzip2::Compression::default(),
        );
        let mut archive = tar::Builder::new(encoder);
        let bytes = b"rootfs";
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "usr/src/marker", &bytes[..])
            .unwrap();
        archive
            .into_inner()
            .unwrap()
            .finish()
            .unwrap()
            .flush()
            .unwrap();
        let nfs = directory.path().join("nfs/device/test");
        let tftp = directory.path().join("tftp/device/test");
        prepare_artifacts(&[image, dtb, rootfs], &nfs, &tftp).unwrap();
        assert_eq!(std::fs::read(nfs.join("usr/src/marker")).unwrap(), bytes);
        assert!(nfs.join(".prep-done").is_file());
        assert_eq!(std::fs::read(tftp.join("Image")).unwrap(), b"image");
        assert_eq!(std::fs::read(tftp.join("board.dtb")).unwrap(), b"dtb");
    }

    #[test]
    fn prepared_event_targets_execution_owner() {
        let event = prepared_event("test-1", "user-1");
        assert_eq!(event.event, "test_execution");
        assert_eq!(event.payload["type"], "queued");
        assert_eq!(event.room.as_deref(), Some("user:user-1"));
    }
}
