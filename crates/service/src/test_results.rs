use std::path::{Component, Path, PathBuf};

use regex::Regex;
use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    config::DeviceConfig,
    events::{ServerEvent, build_performance_events},
    jira::JiraDefect,
    state::AppState,
    test_catalog::TestResultUpdate,
};

#[derive(Debug, thiserror::Error)]
pub enum TestResultError {
    #[error("test result persistence is unavailable")]
    PersistenceUnavailable,
    #[error("invalid test result session")]
    InvalidSession,
    #[error("test execution is no longer accepting results")]
    InactiveExecution,
    #[error("test case was not found in this execution")]
    UnknownCase,
    #[error("invalid test result path")]
    InvalidPath,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct ProcessedTestResult {
    pub test_id: String,
    pub case_id: i64,
    pub build_id: String,
    pub result: String,
    pub comment: String,
    pub version: Option<String>,
    pub completed: bool,
    catalog_results: Vec<CatalogResult>,
}

#[derive(Debug)]
struct CatalogResult {
    run_id: u64,
    case_id: u64,
    result: String,
    comment: String,
    defects: String,
}

#[derive(sqlx::FromRow)]
struct ExecutionTarget {
    build_id: String,
    build_version: Option<String>,
    device_family: String,
    device_type: String,
    device_id: Option<String>,
    created_by: Option<String>,
    test_cycle_id: Option<String>,
    test_plan_name: String,
    name: Option<String>,
}

#[derive(Default)]
struct ArchivedArtifacts {
    output_path: Option<PathBuf>,
    dmesg_path: Option<PathBuf>,
    sources_to_remove: Vec<PathBuf>,
}

pub async fn validate_session(
    pool: &PgPool,
    test_id: &str,
    device_id: &str,
) -> Result<(), TestResultError> {
    let valid = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM test_executions
               WHERE "testId" = $1 AND "deviceId" = $2
                 AND status::text NOT IN ('completed', 'failed', 'cancelled')
                 AND COALESCE("cancelRequested", false) = false
           )"#,
    )
    .bind(test_id)
    .bind(device_id)
    .fetch_one(pool)
    .await?;
    if valid {
        Ok(())
    } else {
        Err(TestResultError::InvalidSession)
    }
}

pub async fn process_case(
    state: &AppState,
    test_id: &str,
    device_id: &str,
    case_id: i64,
) -> Result<ProcessedTestResult, TestResultError> {
    let pool = state
        .database
        .as_ref()
        .map(|database| database.pool())
        .ok_or(TestResultError::PersistenceUnavailable)?;
    let mut transaction = pool.begin().await?;
    let target = lock_execution(&mut transaction, test_id, device_id).await?;
    let output_path = output_path(&state.config.device, &target.device_type, test_id, case_id)?;
    let (output, output_exists) = match tokio::fs::read_to_string(&output_path).await {
        Ok(output) => (output, true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (String::new(), false),
        Err(error) => return Err(error.into()),
    };
    let parsed = parse_output(&output);
    let (title, script_file, suite_name) = sqlx::query_as::<_, (String, String, String)>(
        r#"SELECT title, COALESCE("scriptFile", ''), "suiteName" FROM testcase
           WHERE "executionId" = $1 AND "testCaseId" = $2 FOR UPDATE"#,
    )
    .bind(test_id)
    .bind(case_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(TestResultError::UnknownCase)?;
    let comment = format!(
        "Script file {script_file} executed for test case \"{title}\" with result: {}",
        parsed.result
    );
    let artifacts = archive_artifacts(
        state,
        &target,
        test_id,
        case_id,
        &suite_name,
        &output_path,
        &output,
        output_exists,
    )
    .await;

    let updated = sqlx::query_scalar::<_, i64>(
        r#"UPDATE testcase
           SET result = $4::"enum_testcase_result", comment = $5, cmd = $6,
               "outputFilePath" = $7, "dmesgFilePath" = $8,
               "endedAt" = now(), "updatedAt" = now()
           WHERE "executionId" = $1 AND "testCaseId" = $2
             AND EXISTS (
                 SELECT 1 FROM test_executions execution
                 WHERE execution."testId" = $1 AND execution."deviceId" = $3
             )
           RETURNING "testCaseId"::bigint"#,
    )
    .bind(test_id)
    .bind(case_id)
    .bind(device_id)
    .bind(parsed.result)
    .bind(&comment)
    .bind(parsed.command.as_deref())
    .bind(
        artifacts
            .output_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
    )
    .bind(
        artifacts
            .dmesg_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
    )
    .fetch_optional(&mut *transaction)
    .await?;
    if updated.is_none() {
        transaction.rollback().await?;
        return Err(TestResultError::UnknownCase);
    }

    sqlx::query(
        r#"UPDATE device_action_queue SET "updatedAt" = now()
           WHERE "testId" = $1
             AND status::text NOT IN ('completed', 'cancelled', 'failed')"#,
    )
    .bind(test_id)
    .execute(&mut *transaction)
    .await?;
    insert_log(
        &mut transaction,
        test_id,
        serde_json::json!({
            "message": format!("Test case {case_id} result: {}", parsed.result)
        }),
    )
    .await?;

    let incomplete = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM testcase
               WHERE "executionId" = $1 AND result IS NULL
           )"#,
    )
    .bind(test_id)
    .fetch_one(&mut *transaction)
    .await?;
    if !incomplete {
        complete_execution(&mut transaction, test_id, &target, &state.config.device).await?;
    }
    let mut catalog_results = if incomplete {
        Vec::new()
    } else {
        catalog_results(&mut transaction, &target, test_id).await?
    };
    transaction.commit().await?;
    if parsed.result == "FAIL" {
        sync_jira_defect(
            state,
            pool,
            &target,
            test_id,
            case_id,
            &title,
            &script_file,
            &suite_name,
            &comment,
            &mut catalog_results,
        )
        .await;
    }
    for source in artifacts.sources_to_remove {
        if let Err(error) = tokio::fs::remove_file(&source).await {
            tracing::warn!(%error, path = %source.display(), "failed to remove archived test result source");
        }
    }

    let processed = ProcessedTestResult {
        test_id: test_id.to_owned(),
        case_id,
        build_id: target.build_id,
        result: parsed.result.to_owned(),
        comment,
        version: target.build_version,
        completed: !incomplete,
        catalog_results,
    };
    publish(state, &processed, target.created_by.as_deref());
    Ok(processed)
}

async fn lock_execution(
    transaction: &mut Transaction<'_, Postgres>,
    test_id: &str,
    device_id: &str,
) -> Result<ExecutionTarget, TestResultError> {
    let row = sqlx::query_as::<_, ExecutionTarget>(
        r#"SELECT "buildId" AS build_id, NULLIF("buildVersion", '') AS build_version,
                  "deviceFamily" AS device_family, "deviceType" AS device_type,
                  "deviceId" AS device_id, "createdBy" AS created_by,
                  NULLIF("testCycleId", '') AS test_cycle_id,
                  COALESCE("testPlanName", '') AS test_plan_name, name
           FROM test_executions
           WHERE "testId" = $1 AND "deviceId" = $2
             AND status::text NOT IN ('completed', 'failed', 'cancelled')
             AND COALESCE("cancelRequested", false) = false
           FOR UPDATE"#,
    )
    .bind(test_id)
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(target) = row else {
        return Err(TestResultError::InactiveExecution);
    };
    Ok(target)
}

#[allow(clippy::too_many_arguments)]
async fn sync_jira_defect(
    state: &AppState,
    pool: &PgPool,
    target: &ExecutionTarget,
    test_id: &str,
    case_id: i64,
    title: &str,
    script_file: &str,
    suite_name: &str,
    comment: &str,
    catalog_results: &mut [CatalogResult],
) {
    let Some(jira) = &state.jira else {
        return;
    };
    let defect = JiraDefect {
        summary: title.to_owned(),
        script_file: script_file.to_owned(),
        device_family: target.device_family.clone(),
        build: target.build_id.clone(),
        test_plan: target.test_plan_name.clone(),
        test_suite: suite_name.to_owned(),
        execution_id: test_id.to_owned(),
        test_rail_run: target
            .name
            .clone()
            .or_else(|| target.test_cycle_id.clone())
            .unwrap_or_default(),
        comments: comment.to_owned(),
    };
    let defect_url = match jira.add_defect(&defect).await {
        Ok(defect_url) => defect_url,
        Err(error) => {
            tracing::warn!(%error, %test_id, case_id, "failed to synchronize Jira defect");
            return;
        }
    };
    match sqlx::query(
        r#"UPDATE testcase SET "jiraDefect" = $3, "updatedAt" = now()
           WHERE "executionId" = $1 AND "testCaseId" = $2"#,
    )
    .bind(test_id)
    .bind(case_id)
    .bind(&defect_url)
    .execute(pool)
    .await
    {
        Ok(result) if result.rows_affected() == 1 => {
            if let Some(update) = catalog_results
                .iter_mut()
                .find(|update| update.case_id == case_id as u64)
            {
                update.defects.clone_from(&defect_url);
            }
        }
        Ok(_) => tracing::warn!(%test_id, case_id, "Jira defect testcase row disappeared"),
        Err(error) => tracing::warn!(%error, %test_id, case_id, "failed to persist Jira defect"),
    }
}

async fn catalog_results(
    transaction: &mut Transaction<'_, Postgres>,
    target: &ExecutionTarget,
    test_id: &str,
) -> Result<Vec<CatalogResult>, TestResultError> {
    let Some(run_id) = target
        .test_cycle_id
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return Ok(Vec::new());
    };
    Ok(
        sqlx::query_as::<_, (i64, String, Option<String>, Option<String>)>(
            r#"SELECT "testCaseId"::bigint, result::text, comment, "jiraDefect"
           FROM testcase WHERE "executionId" = $1 ORDER BY id"#,
        )
        .bind(test_id)
        .fetch_all(&mut **transaction)
        .await?
        .into_iter()
        .map(|(case_id, result, comment, defects)| CatalogResult {
            run_id,
            case_id: case_id as u64,
            result,
            comment: comment.unwrap_or_default(),
            defects: defects.unwrap_or_default(),
        })
        .collect(),
    )
}

async fn complete_execution(
    transaction: &mut Transaction<'_, Postgres>,
    test_id: &str,
    target: &ExecutionTarget,
    config: &DeviceConfig,
) -> Result<(), TestResultError> {
    sqlx::query(
        r#"UPDATE test_executions
           SET status = 'completed', "endedAt" = now(), "updatedAt" = now(),
               "executionPhase" = CASE WHEN "deviceId" IS NULL
                   THEN 'COMPLETE_SUCCESS' ELSE 'SEND_FALLBACK_FLASH_REQUEST' END
           WHERE "testId" = $1"#,
    )
    .bind(test_id)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        r#"UPDATE device_action_queue
           SET status = 'completed', "updatedAt" = now()
           WHERE "testId" = $1 AND status::text NOT IN ('cancelled', 'failed')"#,
    )
    .bind(test_id)
    .execute(&mut **transaction)
    .await?;
    let failed = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM testcase
               WHERE "executionId" = $1 AND result::text = 'FAIL'
           )"#,
    )
    .bind(test_id)
    .fetch_one(&mut **transaction)
    .await?;
    if let Some(device_id) = target.device_id.as_deref() {
        let device_update = if failed {
            r#"UPDATE devices SET "lastExecutionStatus" = 'failed', "updatedAt" = now()
               WHERE "deviceId" = $1 AND "lastTestExecution" = $2"#
        } else {
            r#"UPDATE devices SET "lastExecutionStatus" = 'completed', "updatedAt" = now()
               WHERE "deviceId" = $1 AND "lastTestExecution" = $2"#
        };
        sqlx::query(device_update)
            .bind(device_id)
            .bind(test_id)
            .execute(&mut **transaction)
            .await?;
        let fallback_path = PathBuf::from(&config.nfs_host_path)
            .join(&target.device_type)
            .join(target.build_version.as_deref().unwrap_or(&target.build_id));
        sqlx::query(
            r#"INSERT INTO fallback_updates
                  ("deviceId", "releaseId", "testId", "fallbackPath", status,
                   "createdAt", "updatedAt")
               SELECT $1, $2, $3, $4, 'pending', now(), now()
               WHERE NOT EXISTS (
                   SELECT 1 FROM fallback_updates
                   WHERE "testId" = $3 AND status::text IN ('pending', 'flashing', 'completed')
               )"#,
        )
        .bind(device_id)
        .bind(&target.build_id)
        .bind(test_id)
        .bind(fallback_path.to_string_lossy().as_ref())
        .execute(&mut **transaction)
        .await?;
    }
    insert_log(
        transaction,
        test_id,
        serde_json::json!({"message": "Test Execution is marked as completed"}),
    )
    .await
}

async fn insert_log(
    transaction: &mut Transaction<'_, Postgres>,
    test_id: &str,
    data: serde_json::Value,
) -> Result<(), TestResultError> {
    sqlx::query(
        r#"INSERT INTO log_entries
              (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
           VALUES ('test', $1, $2, 'info', now(), now(), now())"#,
    )
    .bind(test_id)
    .bind(data)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn output_path(
    config: &DeviceConfig,
    device_type: &str,
    test_id: &str,
    case_id: i64,
) -> Result<PathBuf, TestResultError> {
    if case_id <= 0 || !safe_component(device_type) || !safe_component(test_id) {
        return Err(TestResultError::InvalidPath);
    }
    Ok(Path::new(&config.nfs_host_path)
        .join(device_type)
        .join(test_id)
        .join("usr/src/tests")
        .join(test_id)
        .join(format!("{case_id}.output")))
}

#[allow(clippy::too_many_arguments)]
async fn archive_artifacts(
    state: &AppState,
    target: &ExecutionTarget,
    test_id: &str,
    case_id: i64,
    suite_name: &str,
    output_source: &Path,
    output: &str,
    output_exists: bool,
) -> ArchivedArtifacts {
    if !output_exists {
        return ArchivedArtifacts::default();
    }
    let version = target.build_version.as_deref().unwrap_or(&target.build_id);
    if !safe_component(&target.device_type) || !safe_component(version) || !safe_component(test_id)
    {
        tracing::warn!(%test_id, "skipping test result archival for unsafe path metadata");
        return ArchivedArtifacts::default();
    }
    let folder = match suite_name.trim().to_ascii_lowercase().as_str() {
        "sanity" => "sanity_commands_results",
        "performance" => "performance_commands_results",
        _ => "results",
    };
    let destination = state
        .config
        .reports
        .test_results_dir
        .join(&target.device_type)
        .join(version)
        .join(test_id)
        .join(folder);
    if let Err(error) = tokio::fs::create_dir_all(&destination).await {
        tracing::warn!(%error, path = %destination.display(), "failed to create test result archive directory");
        return ArchivedArtifacts::default();
    }
    let result_line = Regex::new(r"(?im)^.*\b(?:PASS|FAIL)\b.*$").expect("valid cleanup regex");
    let output_destination = destination.join(format!("test_{case_id}_result.txt"));
    if let Err(error) = tokio::fs::write(
        &output_destination,
        result_line.replace_all(output, "").trim(),
    )
    .await
    {
        tracing::warn!(%error, path = %output_destination.display(), "failed to archive test output");
        return ArchivedArtifacts::default();
    }
    let mut artifacts = ArchivedArtifacts {
        output_path: Some(output_destination),
        dmesg_path: None,
        sources_to_remove: vec![output_source.to_path_buf()],
    };
    let dmesg_source = output_source.with_extension("txt");
    if let Ok(dmesg) = tokio::fs::read(&dmesg_source).await {
        let dmesg_destination = destination.join(format!("test_{case_id}_dmesg.txt"));
        match tokio::fs::write(&dmesg_destination, dmesg).await {
            Ok(()) => {
                artifacts.dmesg_path = Some(dmesg_destination);
                artifacts.sources_to_remove.push(dmesg_source);
            }
            Err(error) => tracing::warn!(%error, "failed to archive testcase dmesg output"),
        }
    }
    artifacts
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value).components().count() == 1
        && matches!(
            Path::new(value).components().next(),
            Some(Component::Normal(_))
        )
}

struct ParsedOutput {
    result: &'static str,
    command: Option<String>,
}

fn parse_output(output: &str) -> ParsedOutput {
    let result_regex = Regex::new(r"(?i)\b(PASS|FAIL)\b").expect("valid result regex");
    let command_regex = Regex::new(r"(?im)^\s*Command:\s*(.+)\s*$").expect("valid command regex");
    let result = result_regex
        .captures(output)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().eq_ignore_ascii_case("PASS"))
        .map_or("FAIL", |passed| if passed { "PASS" } else { "FAIL" });
    let command = command_regex
        .captures(output)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().trim().to_owned());
    ParsedOutput { result, command }
}

fn publish(state: &AppState, result: &ProcessedTestResult, created_by: Option<&str>) {
    let mut events = vec![ServerEvent {
        event: "build_test_execution_update".to_owned(),
        payload: serde_json::json!({
            "buildId": result.build_id,
            "testId": result.test_id,
            "testCaseId": result.case_id.to_string(),
            "status": result.result,
        }),
        room: Some(format!("build:{}", result.build_id)),
    }];
    events.extend(build_performance_events(&result.build_id));
    for event in events {
        if let Err(error) = state.event_publisher.publish(event) {
            tracing::warn!(%error, "failed to publish test result event");
        }
    }
    if let Some(user_id) = created_by {
        let data = if result.completed {
            serde_json::json!({"status": "completed"})
        } else {
            serde_json::json!({"testCaseId": result.case_id, "result": result.result})
        };
        let _ = state.event_publisher.publish(ServerEvent {
            event: "test_execution_update".to_owned(),
            payload: serde_json::json!({
                "buildId": result.build_id,
                "testId": result.test_id,
                "update": {
                    "testId": result.test_id,
                    "type": if result.completed { "status" } else { "testcase" },
                    "data": data,
                },
                "createdBy": user_id,
            }),
            room: Some(format!("user:{user_id}")),
        });
    }
}

pub async fn sync_catalog(state: &AppState, result: &ProcessedTestResult) {
    let Some(catalog) = &state.test_catalog else {
        return;
    };
    for update in &result.catalog_results {
        if let Err(error) = catalog
            .update_result(&TestResultUpdate {
                run_id: update.run_id,
                case_id: update.case_id,
                result: update.result.clone(),
                comment: update.comment.clone(),
                version: result.version.clone(),
                defects: update.defects.clone(),
            })
            .await
        {
            tracing::warn!(%error, test_id = %result.test_id, case_id = update.case_id, "failed to synchronize test result");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_output, safe_component};

    #[test]
    fn parses_case_insensitive_result_and_command() {
        let parsed = parse_output("Command: run-it\nnoise\npass\n");
        assert_eq!(parsed.result, "PASS");
        assert_eq!(parsed.command.as_deref(), Some("run-it"));
    }

    #[test]
    fn missing_result_defaults_to_fail() {
        assert_eq!(parse_output("no result").result, "FAIL");
    }

    #[test]
    fn rejects_traversal_components() {
        assert!(safe_component("gen5"));
        assert!(!safe_component("../gen5"));
        assert!(!safe_component("gen5/type"));
    }
}
