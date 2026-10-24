use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    persistence::entity::{device_action_queue, test_case, test_execution},
    socket::types::{
        farm::ServerEvent,
        frontend::{
            FrontendMessagePayload, TestExecutionUpdateData, TestExecutionUpdateType, TestStatus,
        },
    },
    state::AppState,
    test_catalog::TestRunCreation,
    workers::types::{TestPreparation, TestPreparationCase},
};

use super::artifact_preparation::{artifact_paths, prepare_artifacts, safe_segment};

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseBackend, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, Set, Statement, TransactionTrait,
    sea_query::Expr,
};

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

async fn claim(state: &AppState) -> Result<Option<TestPreparation>, sea_orm::DbErr> {
    let database = state.database.as_ref().expect("worker requires database");
    let lease_seconds =
        i64::try_from(state.config.workers.test_preparation_lease_seconds).unwrap_or(i64::MAX);
    // CTE claim with lease reclaim + release join — keep as raw SQL.
    TestPreparation::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
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
        [lease_seconds.into()],
    ))
    .one(database.orm())
    .await
}

async fn prepare(
    state: &AppState,
    mut preparation: TestPreparation,
) -> Result<ServerEvent, String> {
    let database = state.database.as_ref().expect("worker requires database");
    let cases = test_case::Entity::find()
        .filter(test_case::Column::ExecutionId.eq(&preparation.test_id))
        .order_by_asc(test_case::Column::SuiteId)
        .order_by_asc(test_case::Column::Id)
        .select_only()
        .column_as(test_case::Column::TestCaseId, "test_case_id")
        .column_as(Expr::cust("COALESCE(\"scriptFile\", '')"), "script_file")
        .into_model::<TestPreparationCase>()
        .all(database.orm())
        .await
        .map_err(|error| error.to_string())?;
    let paths = artifact_paths(
        &preparation.artifacts,
        &state.config.device.rootfs_archive_extension,
    )?;
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
        let now = chrono::Utc::now();
        test_execution::Entity::update_many()
            .col_expr(
                test_execution::Column::TestCycleId,
                Expr::value(Some(run_id.clone())),
            )
            .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
            .filter(test_execution::Column::TestId.eq(&preparation.test_id))
            .exec(database.orm())
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
    let transaction = database
        .orm()
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    let now = chrono::Utc::now();
    let existing_queue = device_action_queue::Entity::find()
        .filter(device_action_queue::Column::TestId.eq(&preparation.test_id))
        .one(&transaction)
        .await
        .map_err(|error| error.to_string())?;
    if existing_queue.is_none() {
        let release_id = Uuid::parse_str(&preparation.build_id).ok();
        device_action_queue::ActiveModel {
            id: Set(queue_id),
            test_id: Set(Some(preparation.test_id.clone())),
            device_type: Set(preparation.device_type.clone()),
            user_id: Set(preparation.created_by.clone()),
            r#type: Set(device_action_queue::QueueJobType::Test),
            status: Set(device_action_queue::QueueJobStatus::Queued),
            mode: Set(device_action_queue::QueueDispatchMode::DeviceType),
            release_id: Set(release_id),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&transaction)
        .await
        .map_err(|error| error.to_string())?;
    }
    let persisted_queue_id = device_action_queue::Entity::find()
        .filter(device_action_queue::Column::TestId.eq(&preparation.test_id))
        .order_by_asc(device_action_queue::Column::CreatedAt)
        .one(&transaction)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            format!(
                "device_action_queue row missing for test {}",
                preparation.test_id
            )
        })?
        .id;
    test_execution::Entity::update_many()
        .col_expr(
            test_execution::Column::ExecutionPhase,
            Expr::value(Some(test_execution::ExecutionPhase::QueueTestExecution)),
        )
        .col_expr(
            test_execution::Column::QueueId,
            Expr::value(Some(persisted_queue_id.to_string())),
        )
        .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
        .filter(test_execution::Column::TestId.eq(&preparation.test_id))
        .filter(test_execution::Column::Status.eq(test_execution::TestStatus::NotExecuted))
        .filter(
            sea_orm::Condition::any()
                .add(test_execution::Column::CancelRequested.eq(false))
                .add(test_execution::Column::CancelRequested.is_null()),
        )
        .exec(&transaction)
        .await
        .map_err(|error| error.to_string())?;
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())?;
    let message = format!(
        "Test artifacts prepared and queued for device type {}",
        preparation.device_type
    );
    if let Err(error) =
        crate::devices::test_log::append_test_log(state, &preparation.test_id, &message).await
    {
        tracing::warn!(
            %error,
            test_id = %preparation.test_id,
            "failed to append preparation success log"
        );
    }
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
    let now = chrono::Utc::now();
    let updated = test_execution::Entity::update_many()
        .col_expr(
            test_execution::Column::Status,
            Expr::value(Some(test_execution::TestStatus::Failed)),
        )
        .col_expr(
            test_execution::Column::ExecutionPhase,
            Expr::value(Some(test_execution::ExecutionPhase::CompleteFailed)),
        )
        .col_expr(test_execution::Column::EndedAt, Expr::value(Some(now)))
        .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
        .filter(test_execution::Column::TestId.eq(test_id))
        .exec(database.orm())
        .await
        .ok()?;
    if updated.rows_affected == 0 {
        return None;
    }
    let message = format!("Test preparation failed: {error}");
    if let Err(log_error) = crate::devices::test_log::append_test_log(state, test_id, &message).await
    {
        tracing::warn!(%log_error, %test_id, "failed to append preparation failure log");
    }
    Some(ServerEvent::frontend(FrontendMessagePayload::TestExecution {
        test_id: test_id.to_owned(),
        r#type: TestExecutionUpdateType::Status,
        data: TestExecutionUpdateData {
            status: Some(TestStatus::Failed),
            message: Some(error.to_owned()),
            timestamp: chrono::Utc::now().to_rfc3339(),
            cancel_requested: None,
            phase: None,
            rtos_log_path: None,
        },
    }))
}

fn prepared_event(test_id: &str, _user_id: &str) -> ServerEvent {
    ServerEvent::frontend(FrontendMessagePayload::TestExecution {
        test_id: test_id.to_owned(),
        r#type: TestExecutionUpdateType::Queued,
        data: TestExecutionUpdateData {
            status: Some(TestStatus::NotExecuted),
            timestamp: chrono::Utc::now().to_rfc3339(),
            cancel_requested: None,
            phase: Some("queued".to_owned()),
            rtos_log_path: None,
            message: None,
        },
    })
}

fn record_failure(state: &AppState, error: &sea_orm::DbErr) {
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
    }
}
