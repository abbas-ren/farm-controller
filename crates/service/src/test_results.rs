use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use regex::Regex;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, LockType},
};

use crate::{
    config::DeviceConfig,
    jira::JiraDefect,
    persistence::entity::{
        device, device_action_queue, fallback_update, log_entry, test_case, test_execution,
    },
    socket::{
        helpers::build_performance_event,
        types::{
            farm::ServerEvent,
            frontend::{FrontendMessagePayload, TestStatus},
        },
    },
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
    Database(#[from] sea_orm::DbErr),
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

#[derive(Debug, FromQueryResult)]
struct ExecutionTarget {
    build_id: String,
    build_version: Option<String>,
    device_family: String,
    device_type: String,
    device_id: Option<String>,
    created_by: Option<String>,
    test_cycle_id: Option<String>,
    test_plan_name: Option<String>,
    name: Option<String>,
}

#[derive(Default)]
struct ArchivedArtifacts {
    output_path: Option<PathBuf>,
    dmesg_path: Option<PathBuf>,
    sources_to_remove: Vec<PathBuf>,
}

pub async fn validate_session(
    orm: &DatabaseConnection,
    test_id: &str,
    device_id: &str,
) -> Result<(), TestResultError> {
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "validate_session");
    let count = test_execution::Entity::find()
        .filter(test_execution::Column::TestId.eq(test_id))
        .filter(test_execution::Column::DeviceId.eq(device_id))
        .filter(test_execution::Column::Status.is_not_in([
            test_execution::TestStatus::Completed,
            test_execution::TestStatus::Failed,
            test_execution::TestStatus::Cancelled,
        ]))
        .filter(
            Condition::any()
                .add(test_execution::Column::CancelRequested.eq(false))
                .add(test_execution::Column::CancelRequested.is_null()),
        )
        .count(orm)
        .await?;
    if count > 0 {
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
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "process_case");
    let database = state
        .database
        .as_ref()
        .ok_or(TestResultError::PersistenceUnavailable)?;
    let orm = database.orm();
    let transaction = orm.begin().await?;
    let target = lock_execution(&transaction, test_id, device_id).await?;
    let output_path = output_path(&state.config.device, &target.device_type, test_id, case_id)?;
    let (output, output_exists) = match tokio::fs::read_to_string(&output_path).await {
        Ok(output) => (output, true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (String::new(), false),
        Err(error) => return Err(error.into()),
    };
    let parsed = parse_output(&output);
    #[derive(Debug, FromQueryResult)]
    struct CaseRow {
        title: String,
        script_file: Option<String>,
        suite_name: String,
    }
    let case_row = test_case::Entity::find()
        .filter(test_case::Column::ExecutionId.eq(test_id))
        .filter(test_case::Column::TestCaseId.eq(case_id))
        .lock(LockType::Update)
        .select_only()
        .column(test_case::Column::Title)
        .column(test_case::Column::ScriptFile)
        .column(test_case::Column::SuiteName)
        .into_model::<CaseRow>()
        .one(&transaction)
        .await?
        .ok_or(TestResultError::UnknownCase)?;
    let title = case_row.title;
    let script_file = case_row.script_file.unwrap_or_default();
    let suite_name = case_row.suite_name;
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

    let result_enum = match parsed.result {
        "PASS" => test_case::TestResult::Pass,
        _ => test_case::TestResult::Fail,
    };
    let now = Utc::now();
    let updated = test_case::Entity::update_many()
        .col_expr(test_case::Column::Result, Expr::value(Some(result_enum)))
        .col_expr(test_case::Column::Comment, Expr::value(Some(comment.clone())))
        .col_expr(
            test_case::Column::Cmd,
            Expr::value(parsed.command.clone()),
        )
        .col_expr(
            test_case::Column::OutputFilePath,
            Expr::value(
                artifacts
                    .output_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
            ),
        )
        .col_expr(
            test_case::Column::DmesgFilePath,
            Expr::value(
                artifacts
                    .dmesg_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
            ),
        )
        .col_expr(test_case::Column::EndedAt, Expr::value(Some(now)))
        .col_expr(test_case::Column::UpdatedAt, Expr::value(now))
        .filter(test_case::Column::ExecutionId.eq(test_id))
        .filter(test_case::Column::TestCaseId.eq(case_id))
        .exec(&transaction)
        .await?;
    // Ensure the execution still belongs to this device.
    let execution_ok = test_execution::Entity::find()
        .filter(test_execution::Column::TestId.eq(test_id))
        .filter(test_execution::Column::DeviceId.eq(device_id))
        .count(&transaction)
        .await?;
    if updated.rows_affected == 0 || execution_ok == 0 {
        transaction.rollback().await?;
        return Err(TestResultError::UnknownCase);
    }

    device_action_queue::Entity::update_many()
        .col_expr(
            device_action_queue::Column::UpdatedAt,
            Expr::value(now),
        )
        .filter(device_action_queue::Column::TestId.eq(test_id))
        .filter(device_action_queue::Column::Status.is_not_in([
            device_action_queue::QueueJobStatus::Completed,
            device_action_queue::QueueJobStatus::Cancelled,
        ]))
        .exec(&transaction)
        .await?;
    insert_log(
        &transaction,
        test_id,
        serde_json::json!({
            "message": format!("Test case {case_id} result: {}", parsed.result)
        }),
    )
    .await?;

    let incomplete = test_case::Entity::find()
        .filter(test_case::Column::ExecutionId.eq(test_id))
        .filter(test_case::Column::Result.is_null())
        .count(&transaction)
        .await?
        > 0;
    if !incomplete {
        complete_execution(&transaction, test_id, &target, &state.config.device).await?;
    }
    let mut catalog_results = if incomplete {
        Vec::new()
    } else {
        catalog_results(&transaction, &target, test_id).await?
    };
    transaction.commit().await?;
    if parsed.result == "FAIL" {
        sync_jira_defect(
            state,
            orm,
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
    let log_message = format!("Test case {case_id} result: {}", processed.result);
    if let Err(error) =
        crate::devices::test_log::append_test_log(state, test_id, &log_message).await
    {
        tracing::warn!(%error, %test_id, "failed to append test case result log");
    }
    Ok(processed)
}

async fn lock_execution(
    transaction: &DatabaseTransaction,
    test_id: &str,
    device_id: &str,
) -> Result<ExecutionTarget, TestResultError> {
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "lock_execution");
    let row = test_execution::Entity::find()
        .filter(test_execution::Column::TestId.eq(test_id))
        .filter(test_execution::Column::DeviceId.eq(device_id))
        .filter(test_execution::Column::Status.is_not_in([
            test_execution::TestStatus::Completed,
            test_execution::TestStatus::Failed,
            test_execution::TestStatus::Cancelled,
        ]))
        .filter(
            Condition::any()
                .add(test_execution::Column::CancelRequested.eq(false))
                .add(test_execution::Column::CancelRequested.is_null()),
        )
        .lock(LockType::Update)
        .select_only()
        .column(test_execution::Column::BuildId)
        .column(test_execution::Column::BuildVersion)
        .column(test_execution::Column::DeviceFamily)
        .column(test_execution::Column::DeviceType)
        .column(test_execution::Column::DeviceId)
        .column(test_execution::Column::CreatedBy)
        .column(test_execution::Column::TestCycleId)
        .column(test_execution::Column::TestPlanName)
        .column(test_execution::Column::Name)
        .into_model::<ExecutionTarget>()
        .one(transaction)
        .await?;
    let Some(mut target) = row else {
        return Err(TestResultError::InactiveExecution);
    };
    if target
        .build_version
        .as_deref()
        .is_some_and(str::is_empty)
    {
        target.build_version = None;
    }
    if target
        .test_cycle_id
        .as_deref()
        .is_some_and(str::is_empty)
    {
        target.test_cycle_id = None;
    }
    Ok(target)
}

#[allow(clippy::too_many_arguments)]
async fn sync_jira_defect(
    state: &AppState,
    orm: &DatabaseConnection,
    target: &ExecutionTarget,
    test_id: &str,
    case_id: i64,
    title: &str,
    script_file: &str,
    suite_name: &str,
    comment: &str,
    catalog_results: &mut [CatalogResult],
) {
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "sync_jira_defect");
    let Some(jira) = &state.jira else {
        return;
    };
    let defect = JiraDefect {
        summary: title.to_owned(),
        script_file: script_file.to_owned(),
        device_family: target.device_family.clone(),
        build: target.build_id.clone(),
        test_plan: target.test_plan_name.clone().unwrap_or_default(),
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
    let now = Utc::now();
    match test_case::Entity::update_many()
        .col_expr(
            test_case::Column::JiraDefect,
            Expr::value(Some(defect_url.clone())),
        )
        .col_expr(test_case::Column::UpdatedAt, Expr::value(now))
        .filter(test_case::Column::ExecutionId.eq(test_id))
        .filter(test_case::Column::TestCaseId.eq(case_id))
        .exec(orm)
        .await
    {
        Ok(result) if result.rows_affected == 1 => {
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
    transaction: &DatabaseTransaction,
    target: &ExecutionTarget,
    test_id: &str,
) -> Result<Vec<CatalogResult>, TestResultError> {
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "catalog_results");
    let Some(run_id) = target
        .test_cycle_id
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return Ok(Vec::new());
    };
    #[derive(Debug, FromQueryResult)]
    struct CatalogRow {
        test_case_id: i64,
        result: Option<test_case::TestResult>,
        comment: Option<String>,
        jira_defect: Option<String>,
    }
    Ok(test_case::Entity::find()
        .filter(test_case::Column::ExecutionId.eq(test_id))
        .order_by_asc(test_case::Column::Id)
        .select_only()
        .column(test_case::Column::TestCaseId)
        .column(test_case::Column::Result)
        .column(test_case::Column::Comment)
        .column(test_case::Column::JiraDefect)
        .into_model::<CatalogRow>()
        .all(transaction)
        .await?
        .into_iter()
        .map(|row| CatalogResult {
            run_id,
            case_id: row.test_case_id as u64,
            result: match row.result {
                Some(test_case::TestResult::Pass) => "PASS".to_owned(),
                Some(test_case::TestResult::Fail) => "FAIL".to_owned(),
                None => String::new(),
            },
            comment: row.comment.unwrap_or_default(),
            defects: row.jira_defect.unwrap_or_default(),
        })
        .collect())
}

async fn complete_execution(
    transaction: &DatabaseTransaction,
    test_id: &str,
    target: &ExecutionTarget,
    config: &DeviceConfig,
) -> Result<(), TestResultError> {
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "complete_execution");
    let now = Utc::now();
    let phase = if target.device_id.is_none() {
        test_execution::ExecutionPhase::CompleteSuccess
    } else {
        test_execution::ExecutionPhase::SendFallbackFlashRequest
    };
    test_execution::Entity::update_many()
        .col_expr(
            test_execution::Column::Status,
            Expr::value(Some(test_execution::TestStatus::Completed)),
        )
        .col_expr(test_execution::Column::EndedAt, Expr::value(Some(now)))
        .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
        .col_expr(
            test_execution::Column::ExecutionPhase,
            Expr::value(Some(phase)),
        )
        .filter(test_execution::Column::TestId.eq(test_id))
        .exec(transaction)
        .await?;
    device_action_queue::Entity::update_many()
        .col_expr(
            device_action_queue::Column::Status,
            Expr::value(device_action_queue::QueueJobStatus::Completed),
        )
        .col_expr(
            device_action_queue::Column::UpdatedAt,
            Expr::value(now),
        )
        .filter(device_action_queue::Column::TestId.eq(test_id))
        .filter(device_action_queue::Column::Status.is_not_in([
            device_action_queue::QueueJobStatus::Cancelled,
        ]))
        .exec(transaction)
        .await?;
    let failed = test_case::Entity::find()
        .filter(test_case::Column::ExecutionId.eq(test_id))
        .filter(test_case::Column::Result.eq(test_case::TestResult::Fail))
        .count(transaction)
        .await?
        > 0;
    if let Some(device_id) = target.device_id.as_deref() {
        let status = if failed {
            device::TestStatus::Failed
        } else {
            device::TestStatus::Completed
        };
        device::Entity::update_many()
            .col_expr(
                device::Column::LastExecutionStatus,
                Expr::value(Some(status)),
            )
            .col_expr(device::Column::UpdatedAt, Expr::value(now))
            .filter(device::Column::DeviceId.eq(device_id))
            .filter(device::Column::LastTestExecution.eq(test_id))
            .exec(transaction)
            .await?;
        let fallback_path = PathBuf::from(&config.nfs_host_path)
            .join(&target.device_type)
            .join(target.build_version.as_deref().unwrap_or(&target.build_id));
        let existing = fallback_update::Entity::find()
            .filter(fallback_update::Column::TestId.eq(test_id))
            .filter(fallback_update::Column::Status.is_in([
                fallback_update::FallbackStatus::Pending,
                fallback_update::FallbackStatus::Flashing,
                fallback_update::FallbackStatus::Completed,
            ]))
            .count(transaction)
            .await?;
        if existing == 0 {
            fallback_update::ActiveModel {
                device_id: Set(device_id.to_owned()),
                release_id: Set(target.build_id.clone()),
                test_id: Set(test_id.to_owned()),
                fallback_path: Set(fallback_path.to_string_lossy().into_owned()),
                status: Set(fallback_update::FallbackStatus::Pending),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(transaction)
            .await?;
        }
    }
    insert_log(
        transaction,
        test_id,
        serde_json::json!({"message": "Test Execution is marked as completed"}),
    )
    .await
}

async fn insert_log(
    transaction: &impl ConnectionTrait,
    test_id: &str,
    data: serde_json::Value,
) -> Result<(), TestResultError> {
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "insert_log");
    let now = Utc::now();
    log_entry::ActiveModel {
        r#type: Set(log_entry::LogEntryType::Test),
        reference_id: Set(test_id.to_owned()),
        data: Set(data),
        level: Set(log_entry::LogEntryLevel::Info),
        timestamp: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}

fn output_path(
    config: &DeviceConfig,
    device_type: &str,
    test_id: &str,
    case_id: i64,
) -> Result<PathBuf, TestResultError> {
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "output_path");
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
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "archive_artifacts");
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
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "safe_component");
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
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "parse_output");
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

fn publish(state: &AppState, result: &ProcessedTestResult, _created_by: Option<&str>) {
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "publish");
    let mut events = vec![ServerEvent::frontend(
        FrontendMessagePayload::BuildTestExecutionUpdate {
            test_id: result.test_id.clone(),
            build_id: result.build_id.clone(),
            test_case_id: Some(result.case_id.to_string()),
            status: Some(result.result.clone()),
        },
    )];

    events.push(ServerEvent::frontend(
        FrontendMessagePayload::TestExecutionUpdate {
            build_id: Some(result.build_id.clone()),
            test_id: result.test_id.clone(),
            status: Some(if result.completed {
                TestStatus::Completed
            } else {
                TestStatus::InProgress
            }),
            progress: if result.completed { Some(100) } else { None },
            message: None,
            cancel_requested: None,
        },
    ));

    events.push(build_performance_event(&result.build_id));

    for event in events {
        if let Err(error) = state.event_publisher.publish(event) {
            tracing::warn!(%error, "failed to publish test result event");
        }
    }
}

pub async fn sync_catalog(state: &AppState, result: &ProcessedTestResult) {
    let _api_trace = crate::observability::ApiTrace::enter("test_results", "sync_catalog");
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
        let _api_trace = crate::observability::ApiTrace::enter(
            "test_results",
            "parses_case_insensitive_result_and_command",
        );
        let parsed = parse_output("Command: run-it\nnoise\npass\n");
        assert_eq!(parsed.result, "PASS");
        assert_eq!(parsed.command.as_deref(), Some("run-it"));
    }

    #[test]
    fn missing_result_defaults_to_fail() {
        let _api_trace =
            crate::observability::ApiTrace::enter("test_results", "missing_result_defaults_to_fail");
        assert_eq!(parse_output("no result").result, "FAIL");
    }

    #[test]
    fn rejects_traversal_components() {
        let _api_trace =
            crate::observability::ApiTrace::enter("test_results", "rejects_traversal_components");
        assert!(safe_component("gen5"));
        assert!(!safe_component("../gen5"));
        assert!(!safe_component("gen5/type"));
    }
}
