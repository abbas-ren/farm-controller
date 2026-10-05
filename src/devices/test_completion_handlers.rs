use std::{
    path::{Component, Path as FsPath, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{error::ErrorResponse, events::ServerEvent, state::AppState};

use super::{
    EDGE_CONTROLLER_PORT, TestCompletionQuery, TestCompletionResponse, error_response,
    repository_error, repository_types::TestCompletionTarget,
};

const RTOS_END_TIMEOUT: Duration = Duration::from_secs(60);

#[utoipa::path(
    get,
    path = "/api/v1/device/{id}/test-completed",
    tag = "Devices",
    summary = "Handle a device test completion callback",
    description = "Public device callback. Terminal executions are acknowledged unchanged; active Gen4/Gen5 executions best-effort collect RTOS output, persist its path, and notify the execution owner.",
    params(("id" = String, Path, description = "Device ID"), TestCompletionQuery),
    responses(
        (status = 200, description = "Test completion handled", body = TestCompletionResponse),
        (status = 400, description = "Missing or empty test ID", body = ErrorResponse),
        (status = 404, description = "Test execution not found", body = ErrorResponse),
        (status = 500, description = "Test completion lookup failed", body = ErrorResponse),
        (status = 503, description = "Device persistence unavailable", body = ErrorResponse)
    )
)]
pub async fn test_completed(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
    Query(query): Query<TestCompletionQuery>,
) -> axum::response::Response {
    let test_id = query
        .test_id
        .chars()
        .filter(|character| !matches!(character, '\'' | '"') && !character.is_whitespace())
        .collect::<String>();
    if test_id.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "testId must not be empty",
        );
    }
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository
        .test_completion_target(&test_id, &device_id)
        .await
    {
        Ok(Some(target)) => {
            if !is_terminal(&target.status)
                && let Err(error) = collect_rtos_log(&state, &test_id, &target).await
            {
                tracing::warn!(%error, %device_id, %test_id, "best-effort RTOS log collection failed");
            }
            Json(serde_json::json!({
                "success": true,
                "message": format!("Test completion handled for device {device_id}")
            }))
            .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &format!("Test execution {test_id} not found"),
        ),
        Err(error) => repository_error(error, "Failed to handle test completion"),
    }
}

async fn collect_rtos_log(
    state: &AppState,
    test_id: &str,
    target: &TestCompletionTarget,
) -> Result<(), String> {
    let Some((controller_ip, payload)) = rtos_request(target) else {
        return Ok(());
    };
    let url = reqwest::Url::parse(&format!(
        "http://{controller_ip}:{EDGE_CONTROLLER_PORT}/rtos/end"
    ))
    .map_err(|error| error.to_string())?;
    let client =
        crate::external_http::client(RTOS_END_TIMEOUT).map_err(|error| error.to_string())?;
    let content = request_rtos_log(&client, url, &payload).await?;
    if content.is_empty() {
        return Ok(());
    }
    let path = rtos_log_path(&state.config.reports.test_results_dir, target, test_id)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
    }
    tokio::fs::write(&path, content)
        .await
        .map_err(|error| error.to_string())?;
    let path = path
        .to_str()
        .ok_or_else(|| "RTOS log path is not valid UTF-8".to_owned())?;
    let repository = state
        .device_repository
        .as_ref()
        .ok_or_else(|| "device persistence is unavailable".to_owned())?;
    repository
        .store_rtos_log_path(test_id, path)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(user_id) = target.created_by.as_deref() {
        let _ = state.event_publisher.publish(rtos_log_event(
            test_id,
            target.build_id.as_deref(),
            path,
            user_id,
        ));
    }
    Ok(())
}

fn rtos_request(target: &TestCompletionTarget) -> Option<(&str, serde_json::Value)> {
    let family = target
        .device_family
        .as_deref()?
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    let mac = target.mac_address.as_deref()?;
    match family.as_str() {
        "gen5" => Some((
            target.gen5_controller_ip.as_deref()?,
            serde_json::json!({
                "mac": mac,
                "gen": 5,
                "rtos": next_uart_port(target.uart_port.as_deref()?),
            }),
        )),
        "gen4" => Some((
            target.gen4_controller_ip.as_deref()?,
            serde_json::json!({
                "mac": mac,
                "gen": 4,
                "serial": target.relay_serial.as_deref()?,
                "channel": target.relay_channel?,
            }),
        )),
        _ => None,
    }
}

async fn request_rtos_log(
    client: &reqwest::Client,
    url: reqwest::Url,
    payload: &serde_json::Value,
) -> Result<Vec<u8>, String> {
    let response = client
        .post(url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .json(payload)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    Ok(response
        .bytes()
        .await
        .map_err(|error| error.to_string())?
        .to_vec())
}

fn rtos_log_path(
    root: &FsPath,
    target: &TestCompletionTarget,
    test_id: &str,
) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    for segment in [
        target.device_type.as_str(),
        target.build_version.as_deref().unwrap_or_default(),
        test_id,
    ] {
        if segment.is_empty() {
            continue;
        }
        if FsPath::new(segment).components().count() != 1
            || !matches!(
                FsPath::new(segment).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err(format!("invalid RTOS artifact path segment: {segment}"));
        }
        path.push(segment);
    }
    Ok(path.join("rtosLogFile.txt"))
}

fn next_uart_port(port: &str) -> String {
    let split = port
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_ascii_digit())
        .map_or(0, |(index, character)| index + character.len_utf8());
    let (prefix, suffix) = port.split_at(split);
    suffix.parse::<u32>().map_or_else(
        |_| port.to_owned(),
        |number| format!("{prefix}{}", number + 1),
    )
}

fn is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "cancelled" | "failed")
}

fn rtos_log_event(test_id: &str, build_id: Option<&str>, path: &str, user_id: &str) -> ServerEvent {
    ServerEvent {
        event: "test_execution".to_owned(),
        payload: serde_json::json!({
            "testId": test_id,
            "type": "rtos_log",
            "data": {
                "rtosLogPath": path,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            },
            "buildId": build_id,
        }),
        room: Some(format!("user:{user_id}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> TestCompletionTarget {
        TestCompletionTarget {
            status: "in_progress".to_owned(),
            device_family: Some("Gen 5".to_owned()),
            mac_address: Some("00:11:22:33:44:55".to_owned()),
            device_type: "x5h".to_owned(),
            build_version: Some("v1.2.3".to_owned()),
            build_id: Some("build-1".to_owned()),
            created_by: Some("user-1".to_owned()),
            gen5_controller_ip: Some("192.0.2.10".to_owned()),
            uart_port: Some("/dev/ttyUSB1".to_owned()),
            gen4_controller_ip: None,
            relay_serial: None,
            relay_channel: None,
        }
    }

    #[test]
    fn gen5_request_uses_the_adjacent_rtos_uart() {
        let target = target();
        let (controller, payload) = rtos_request(&target).unwrap();
        assert_eq!(controller, "192.0.2.10");
        assert_eq!(payload["gen"], 5);
        assert_eq!(payload["rtos"], "/dev/ttyUSB2");
    }

    #[test]
    fn rtos_artifact_path_is_scoped_and_rejects_traversal() {
        let root = FsPath::new("/test-results");
        assert_eq!(
            rtos_log_path(root, &target(), "test-1").unwrap(),
            PathBuf::from("/test-results/x5h/v1.2.3/test-1/rtosLogFile.txt")
        );
        assert!(rtos_log_path(root, &target(), "../test-1").is_err());
    }

    #[test]
    fn rtos_event_targets_the_execution_owner() {
        let event = rtos_log_event("test-1", Some("build-1"), "/tmp/rtos.log", "user-1");
        assert_eq!(event.event, "test_execution");
        assert_eq!(event.payload["type"], "rtos_log");
        assert_eq!(event.payload["data"]["rtosLogPath"], "/tmp/rtos.log");
        assert_eq!(event.room.as_deref(), Some("user:user-1"));
    }
}
