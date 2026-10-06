//! Create and update test execution commands.

use super::*;
use crate::{auth, error::ErrorResponse, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use std::sync::Arc;

#[utoipa::path(post, path = "/api/v1/device/test/execution", tag = "Tests", summary = "Create a test execution", description = "Expands the requested TestRail selection, atomically persists the execution and cases, and queues artifact preparation after commit.", security(("bearer_auth" = [])), request_body = CreateExecutionRequest, responses((status = 201, description = "Complete legacy test-execution row", body = Object), (status = 400, description = "Invalid execution selection", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Build not found", body = ErrorResponse), (status = 500, description = "Execution creation failed", body = ErrorResponse)))]
pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateExecutionRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let user_id = auth::token_subject(&headers).unwrap_or_else(|| "system".to_owned());
    match create_for_user(&state, request, user_id).await {
        Ok(created) => (StatusCode::CREATED, response_headers, Json(created)).into_response(),
        Err(error) => repository_error(error, "Failed to create test execution"),
    }
}

pub(crate) async fn create_for_user(
    state: &AppState,
    request: CreateExecutionRequest,
    user_id: String,
) -> Result<serde_json::Value, super::super::error::DeviceRepositoryError> {
    use super::super::error::DeviceRepositoryError;

    let execution = super::selection::resolve_execution(state, request, user_id).await?;
    let repository = state.device_repository.as_ref().ok_or_else(|| {
        DeviceRepositoryError::Internal("Device persistence is unavailable".to_owned())
    })?;
    let created = repository.create_test_execution(&execution).await?;
    if let Some(test_id) = created.get("testId").and_then(serde_json::Value::as_str) {
        let event = super::events::new_execution_event(test_id, &execution.user_id);
        if let Err(error) = state.event_publisher.publish(event) {
            tracing::warn!(%error, test_id, "failed to publish new test execution");
        }
    }
    Ok(created)
}

#[utoipa::path(put, path = "/api/v1/device/test/execution/{id}", tag = "Tests", summary = "Update a TestRail case result", description = "Preserves the legacy best-effort contract: TestRail failures are logged and the request still receives an empty 201 acknowledgement.", params(("id" = String, Path, description = "Test case identifier")), security(("bearer_auth" = [])), request_body = UpdateExecutionRequest, responses((status = 201, description = "Update processed", body = EmptyObjectResponse), (status = 400, description = "Invalid update", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse)))]
pub async fn update(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(case_id): Path<String>,
    Json(request): Json<UpdateExecutionRequest>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    if request.test_id.trim().is_empty()
        || !super::validation::ALLOWED_RESULTS.contains(&request.result.as_str())
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "Invalid test execution update",
        );
    }
    let operation = async {
        let run_id = request
            .test_id
            .parse::<u64>()
            .map_err(|_| "Test ID must be numeric".to_owned())?;
        let case_id = case_id
            .parse::<u64>()
            .map_err(|_| "Test case ID must be numeric".to_owned())?;
        let mut version = request.build_id.clone();
        if let Some(repository) = &state.device_repository {
            version = repository
                .test_execution_build_version(&request.test_id)
                .await
                .map_err(|error| error.to_string())?
                .or(version);
        }
        let catalog = state
            .test_catalog
            .as_ref()
            .ok_or_else(|| "TestRail is not configured".to_owned())?;
        catalog
            .update_result(&crate::test_catalog::TestResultUpdate {
                run_id,
                case_id,
                result: request.result.clone(),
                comment: request.comments.clone(),
                version,
                defects: String::new(),
            })
            .await
            .map_err(|error| error.to_string())
    }
    .await;
    if let Err(error) = operation {
        tracing::warn!(%error, test_id = %request.test_id, "failed to update TestRail result");
        if request.log
            && let Some(repository) = &state.device_repository
        {
            let _ = repository
                .create_log(&LogCreateRequest {
                    log_type: "test".to_owned(),
                    reference_id: request.test_id.clone(),
                    data: serde_json::json!({"message": error}),
                    level: "error".to_owned(),
                    timestamp: None,
                })
                .await;
        }
    }
    (
        StatusCode::CREATED,
        response_headers,
        Json(serde_json::json!({})),
    )
        .into_response()
}
