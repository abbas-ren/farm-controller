//! Persist test-execution progress lines and publish them to the frontend.
//!
//! Shared by HTTP handlers, workers, and runtime log ingestion. Replaces Node test-log
//! append paths used during execution progress updates in `farm_controller_old`.

use std::sync::OnceLock;

use tokio::sync::Mutex;

use crate::{
    socket::types::{
        farm::{EventHub, ServerEvent},
        frontend::{
            FrontendMessagePayload, TestExecutionUpdateData, TestExecutionUpdateType,
        },
    },
    state::AppState,
};

static LOG_FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// PURPOSE: Helper `log_file_lock` supporting device HTTP handlers (log file lock).
/// USE: Helper used by sibling device HTTP handlers.
fn log_file_lock() -> &'static Mutex<()> {
    LOG_FILE_LOCK.get_or_init(|| Mutex::new(()))
}

/// PURPOSE: Append a progress line to `{test_logs_dir}/{testId}.json` and emit a `testExecution` / `log` event.
/// USE: publishes Socket.IO frontend events; also used by workers / other handlers.
/// Append a progress line to `{test_logs_dir}/{testId}.json` and emit a `testExecution` / `log` event.
pub(crate) async fn append_test_log(
    state: &AppState,
    test_id: &str,
    message: &str,
) -> Result<(), String> {
    let timestamp = chrono::Utc::now().to_rfc3339();
    write_test_log_file(
        &state.config.tests.test_logs_dir,
        test_id,
        message,
        &timestamp,
    )
    .await?;
    publish_test_log(&state.event_publisher, test_id, message, &timestamp)
}

/// PURPOSE: Helper `write_test_log_file` supporting device HTTP handlers (write test log file).
/// USE: Helper used by sibling device HTTP handlers.
pub(crate) async fn write_test_log_file(
    logs_dir: &str,
    test_id: &str,
    message: &str,
    timestamp: &str,
) -> Result<(), String> {
    let _guard = log_file_lock().lock().await;
    let dir = std::path::Path::new(logs_dir);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| error.to_string())?;
    let path = dir.join(format!("{test_id}.json"));
    let mut logs: Vec<String> = match tokio::fs::read_to_string(&path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.to_string()),
    };
    logs.push(format!("[{timestamp}] {message}"));
    let body = serde_json::to_vec_pretty(&logs).map_err(|error| error.to_string())?;
    tokio::fs::write(path, body)
        .await
        .map_err(|error| error.to_string())
}

/// PURPOSE: Helper `test_log_event` supporting device HTTP handlers (test log event).
/// USE: publishes Socket.IO frontend events.
pub(crate) fn test_log_event(test_id: &str, message: &str, timestamp: &str) -> ServerEvent {
    ServerEvent::frontend(FrontendMessagePayload::TestExecution {
        test_id: test_id.to_owned(),
        r#type: TestExecutionUpdateType::Log,
        data: TestExecutionUpdateData {
            status: None,
            message: Some(message.to_owned()),
            timestamp: timestamp.to_owned(),
            cancel_requested: None,
            phase: None,
            rtos_log_path: None,
        },
    })
}

/// PURPOSE: Helper `publish_test_log` supporting device HTTP handlers (publish test log).
/// USE: Helper used by sibling device HTTP handlers.
fn publish_test_log(
    publisher: &EventHub,
    test_id: &str,
    message: &str,
    timestamp: &str,
) -> Result<(), String> {
    publisher.publish(test_log_event(test_id, message, timestamp))
}
