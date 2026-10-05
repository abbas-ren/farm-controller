use axum::http::HeaderMap;

use crate::{auth, events::ServerEvent, state::AppState};

use super::repository_types::{BuildUploadFinalization, BuildUploadResult};

pub(crate) fn user_id(headers: &HeaderMap) -> String {
    headers
        .get("x-user-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| auth::token_subject(headers))
        .unwrap_or_else(|| "system".to_owned())
}

pub(crate) fn progress(state: &AppState, user_id: &str, upload_id: &str, percent: u64) {
    publish(
        state,
        ServerEvent {
            event: "upload_progress".to_owned(),
            payload: serde_json::json!({"uploadId": upload_id, "percent": percent.min(100)}),
            room: Some(format!("user:{user_id}")),
        },
    );
}

pub(crate) fn failure(state: &AppState, user_id: &str, upload_id: &str, message: &str) {
    publish(
        state,
        ServerEvent {
            event: "upload_error".to_owned(),
            payload: serde_json::json!({"uploadId": upload_id, "message": message}),
            room: Some(format!("user:{user_id}")),
        },
    );
}

pub(crate) async fn completed(
    state: &AppState,
    request: &BuildUploadFinalization,
    result: BuildUploadResult,
) {
    if let Err(error) =
        crate::workers::prepare_uploaded_build(state, &result.release, &request.user_id).await
    {
        state
            .metrics
            .record_worker_run("build_preparation", "failure");
        tracing::error!(%error, upload_id = %request.upload_id, "failed to prepare uploaded build");
    } else {
        state
            .metrics
            .record_worker_run("build_preparation", "success");
    }
    let mut complete = serde_json::json!({
        "uploadId": request.upload_id,
        "fileName": request.filename,
    });
    if let Some(tag) = request.tag.as_deref().filter(|tag| !tag.trim().is_empty()) {
        complete["tag"] = serde_json::Value::String(tag.to_owned());
    }
    publish(
        state,
        ServerEvent {
            event: "upload_complete".to_owned(),
            payload: complete,
            room: Some(format!("user:{}", request.user_id)),
        },
    );
    publish(
        state,
        ServerEvent {
            event: "build_uploaded".to_owned(),
            payload: serde_json::json!({
                "releaseId": result.release["id"],
                "version": result.release["version"],
                "deviceType": result.release["deviceType"],
                "deviceFamily": result.release["deviceFamily"],
                "buildType": result.release["buildType"],
            }),
            room: None,
        },
    );
    publish(state, ServerEvent::alert("device-alert", result.alert));
}

fn publish(state: &AppState, event: ServerEvent) {
    if let Err(error) = state.event_publisher.publish(event) {
        tracing::warn!(%error, "failed to publish build upload event");
    }
}
