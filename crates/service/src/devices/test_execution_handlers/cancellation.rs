//! Cancellation command, deferred completion, and cancellation event construction.

use super::*;
use crate::{
    auth,
    devices::{EDGE_CONTROLLER_PORT, TEST_CANCELLATION_TIMEOUT},
    error::ErrorResponse,
    events::ServerEvent,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use std::sync::Arc;

#[utoipa::path(put, path = "/api/v1/device/test/cancel/{id}", tag = "Tests", summary = "Request test execution cancellation", description = "Persists an idempotent cancellation request before publishing owner and build-room events.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Cancellation requested", body = MessageResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Cancellation failed or execution not found", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn cancel(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let user_id = auth::token_subject(&headers).unwrap_or_else(|| "system".to_owned());
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository
        .request_test_cancellation(&test_id, &user_id, &state.config.device.nfs_host_path)
        .await
    {
        Ok(Some(result)) => {
            if result.changed {
                for event in cancellation_events(
                    &test_id,
                    &result.build_id,
                    &result.phase,
                    &result.created_by,
                ) {
                    if let Err(error) = state.event_publisher.publish(event) {
                        tracing::warn!(%error, test_id, "failed to publish test cancellation");
                    }
                }
            }
            if result.terminalized {
                if let Some(device_ip) = result.device_ip.as_deref() {
                    send_device_cancellation(device_ip, &test_id).await;
                }
                if let Some(device_type) = result.cleanup_device_type.as_deref() {
                    cleanup_cancelled_preparation(&state, device_type, &test_id).await;
                    if let Some(run_id) = result
                        .test_cycle_id
                        .as_deref()
                        .and_then(|value| value.parse::<u64>().ok())
                        && let Some(catalog) = state.test_catalog.as_ref()
                        && let Err(error) = catalog.delete_run(run_id).await
                    {
                        tracing::warn!(%error, run_id, test_id, "failed to delete cancelled TestRail run");
                    }
                }
                if let Err(error) = state.event_publisher.publish(ServerEvent {
                    event: "test_execution_update".to_owned(),
                    payload: serde_json::json!({
                        "testId": test_id,
                        "buildId": result.build_id,
                        "update": {
                            "testId": test_id,
                            "type": "status",
                            "data": {
                                "status": "cancelled",
                                "timestamp": chrono::Utc::now().to_rfc3339(),
                            }
                        },
                        "createdBy": result.created_by,
                    }),
                    room: Some(format!("user:{}", result.created_by)),
                }) {
                    tracing::warn!(%error, test_id, "failed to publish terminal cancellation");
                }
            }
            (
                response_headers,
                Json(serde_json::json!({"message": "  Test execution cancelled successfully"})),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Could not cancel test execution",
        ),
        Err(error) => repository_error(error, "Error while cancelling test execution"),
    }
}

async fn send_device_cancellation(device_ip: &str, test_id: &str) {
    let Ok(url) = reqwest::Url::parse(&format!("http://{device_ip}:{EDGE_CONTROLLER_PORT}/cancel"))
    else {
        tracing::warn!(
            device_ip,
            test_id,
            "invalid device address for cancellation"
        );
        return;
    };
    let client = match crate::external_http::client(TEST_CANCELLATION_TIMEOUT) {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, test_id, "failed to create cancellation client");
            return;
        }
    };
    if let Err(error) = client
        .post(url)
        .json(&serde_json::json!({"testId": test_id}))
        .send()
        .await
    {
        tracing::warn!(%error, test_id, "device cancellation request failed");
    }
}

async fn cleanup_cancelled_preparation(state: &AppState, device_type: &str, test_id: &str) {
    if !super::validation::safe_segment(device_type) || !super::validation::safe_segment(test_id) {
        tracing::warn!(
            device_type,
            test_id,
            "refusing unsafe cancelled-test cleanup path"
        );
        return;
    }
    let path = std::path::Path::new(&state.config.device.nfs_host_path)
        .join(device_type)
        .join(test_id);
    if let Err(error) = tokio::fs::remove_dir_all(&path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %path.display(), test_id, "failed to remove cancelled test artifacts");
    }
}

pub(crate) async fn complete_deferred_cancellation(
    state: &AppState,
    cancellation: &super::super::repository_types::DeferredCancellation,
) {
    if let Err(error) = state.event_publisher.publish(ServerEvent {
        event: "test_execution_update".to_owned(),
        payload: serde_json::json!({
            "testId": cancellation.test_id,
            "buildId": cancellation.build_id,
            "update": {
                "testId": cancellation.test_id,
                "type": "status",
                "data": {
                    "status": "cancelled",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                }
            },
            "createdBy": cancellation.created_by,
        }),
        room: Some(format!("user:{}", cancellation.created_by)),
    }) {
        tracing::warn!(
            %error,
            test_id = cancellation.test_id,
            "failed to publish deferred terminal cancellation"
        );
    }
}

pub(crate) fn cancellation_events(
    test_id: &str,
    build_id: &str,
    phase: &str,
    user_id: &str,
) -> [ServerEvent; 2] {
    let timestamp = chrono::Utc::now().to_rfc3339();
    [
        ServerEvent {
            event: "test_execution".to_owned(),
            payload: serde_json::json!({
                "testId": test_id,
                "type": "cancel_requested",
                "data": {
                    "cancelRequested": true,
                    "phase": phase,
                    "timestamp": timestamp,
                },
            }),
            room: Some(format!("user:{user_id}")),
        },
        ServerEvent {
            event: "test_execution_update".to_owned(),
            payload: serde_json::json!({
                "testId": test_id,
                "buildId": build_id,
                "cancelRequested": true,
            }),
            room: Some(format!("build:{build_id}")),
        },
    ]
}
