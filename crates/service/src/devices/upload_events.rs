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

pub(crate) fn progress(state: &AppState, _user_id: &str, upload_id: &str, percent: u64) {
    publish(
        state,
        ServerEvent::frontend(FrontendMessagePayload::UploadProgress {
            upload_id: upload_id.to_owned(),
            percent: percent.min(100),
        }),
    );
}

pub(crate) fn failure(state: &AppState, _user_id: &str, upload_id: &str, message: &str) {
    publish(
        state,
        ServerEvent::frontend(FrontendMessagePayload::UploadError {
            upload_id: upload_id.to_owned(),
            message: message.to_owned(),
        }),
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

fn publish(state: &AppState, event: ServerEvent) {
    if let Err(error) = state.event_publisher.publish(event) {
        tracing::warn!(%error, "failed to publish build upload event");
    }
}
