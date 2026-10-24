//! Frontend socket events for build upload progress, completion, and failure.
//!
//! Called from TUS / multipart build upload handlers after each meaningful
//! stage. Replaces the Socket.IO emit paths in Node `BuildService` upload
//! flows so the UI can track `uploadId` progress in real time.

use axum::http::HeaderMap;

use crate::{
    auth,
    socket::types::{
        farm::ServerEvent,
        frontend::{AlertLevel, FrontendMessagePayload},
    },
    state::AppState,
};

use super::repository_types::{BuildUploadFinalization, BuildUploadResult};

/// PURPOSE: Resolve the acting user id from headers or JWT subject.
/// USE: Upload handlers stamp `created_by` / event payloads; falls back to `"system"`.
pub(crate) fn user_id(headers: &HeaderMap) -> String {
    let _api_trace = crate::observability::ApiTrace::enter("devices::upload_events", "user_id");
    headers
        .get("x-user-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| auth::token_subject(headers))
        .unwrap_or_else(|| "system".to_owned())
}

/// PURPOSE: Emit an upload progress percentage to the frontend event hub.
/// USE: PATCH / TUS chunk handlers call this as bytes land on disk.
pub(crate) fn progress(state: &AppState, _user_id: &str, upload_id: &str, percent: u64) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::upload_events", "progress");
    publish(
        state,
        ServerEvent::frontend(FrontendMessagePayload::UploadProgress {
            upload_id: upload_id.to_owned(),
            percent: percent.min(100),
        }),
    );
}

/// PURPOSE: Emit an upload error payload when finalization or transfer fails.
/// USE: Upload handlers after marking the upload row failed in the repository.
pub(crate) fn failure(state: &AppState, _user_id: &str, upload_id: &str, message: &str) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::upload_events", "failure");
    publish(
        state,
        ServerEvent::frontend(FrontendMessagePayload::UploadError {
            upload_id: upload_id.to_owned(),
            message: message.to_owned(),
        }),
    );
}

/// PURPOSE: Kick off build preparation and notify the UI that an upload finished.
/// USE: Called after `finalize_build_upload` succeeds; also publishes `BuildUploaded` and an alert.
pub(crate) async fn completed(
    state: &AppState,
    request: &BuildUploadFinalization,
    result: BuildUploadResult,
) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::upload_events", "completed");
    // Mirror Node BuildService: preparation runs asynchronously after DB finalize.
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

    publish(
        state,
        ServerEvent::frontend(FrontendMessagePayload::UploadComplete {
            upload_id: request.upload_id.to_owned(),
            file_name: request.filename.to_owned(),
            tag: if let Some(tag) = request.tag.as_deref().filter(|tag| !tag.trim().is_empty()) {
                Some(tag.to_owned())
            } else {
                None
            },
        }),
    );
    publish(
        state,
        ServerEvent::frontend(FrontendMessagePayload::BuildUploaded {
            release_id: result.release["id"].as_str().unwrap_or_default().to_owned(),
            version: result.release["version"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            device_type: result.release["deviceType"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            device_family: result.release["deviceFamily"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            build_type: result.release["buildType"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        }),
    );
    publish(
        state,
        ServerEvent::frontend(FrontendMessagePayload::AlertDataPayload {
            level: Some(AlertLevel::Critical),
            subtype: Some("error".to_owned()),
            message: result.alert,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        }),
    );
}

/// PURPOSE: Publish a server event, logging (not failing) on hub errors.
/// USE: Internal helper for all upload event emits.
fn publish(state: &AppState, event: ServerEvent) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::upload_events", "publish");
    if let Err(error) = state.event_publisher.publish(event) {
        tracing::warn!(%error, "failed to publish build upload event");
    }
}
