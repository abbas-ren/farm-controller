use super::{
    ActiveExecutionRecord, BuildExecutionList, CreateExecutionRequest, EmptyObjectResponse,
    ExecutionCaseInput, ExecutionCaseRecord, ExecutionList, ExecutionListQuery,
    ExecutionReportRecord, ExecutionSelection, LogCreateRequest, MessageResponse,
    UpdateExecutionRequest, error_response, repository_error, repository_types::ExecutionCreation,
};

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
) -> Result<serde_json::Value, super::error::DeviceRepositoryError> {
    use super::error::DeviceRepositoryError;

    let execution = resolve_execution(state, request, user_id).await?;
    let repository = state.device_repository.as_ref().ok_or_else(|| {
        DeviceRepositoryError::Internal("Device persistence is unavailable".to_owned())
    })?;
    let created = repository.create_test_execution(&execution).await?;
    if let Some(test_id) = created.get("testId").and_then(serde_json::Value::as_str) {
        let event = new_execution_event(test_id, &execution.user_id);
        if let Err(error) = state.event_publisher.publish(event) {
            tracing::warn!(%error, test_id, "failed to publish new test execution");
        }
    }
    Ok(created)
}

pub(crate) fn new_execution_event(test_id: &str, user_id: &str) -> ServerEvent {
    ServerEvent {
        event: "test_execution".to_owned(),
        payload: serde_json::json!({
            "testId": test_id,
            "type": "new",
            "data": {
                "status": "not_executed",
                "message": format!("Test execution {test_id} created"),
                "timestamp": chrono::Utc::now().to_rfc3339(),
            },
        }),
        room: Some(format!("user:{user_id}")),
    }
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
        || !matches!(
            request.result.as_str(),
            "PASS" | "FAIL" | "BLOCKED" | "RETEST" | "NOT EXECUTED"
        )
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

async fn resolve_execution(
    state: &AppState,
    request: CreateExecutionRequest,
    user_id: String,
) -> Result<ExecutionCreation, super::error::DeviceRepositoryError> {
    use super::error::DeviceRepositoryError;

    if request.device_type.trim().is_empty() {
        return Err(DeviceRepositoryError::Validation(
            "Device type is required".to_owned(),
        ));
    }
    if request.build_id.trim().is_empty() {
        return Err(DeviceRepositoryError::Validation(
            "Build ID is required".to_owned(),
        ));
    }
    let selection = request.selection.clone();
    let mut plan_name = request.test_plan_name.clone().unwrap_or_default();
    let mut suites = request.test_suites.clone();
    let mut is_all_selected = request.is_all_selected;
    let mut cases = Vec::new();
    let plan_id = match &selection {
        Some(ExecutionSelection::All {
            plan_id,
            plan_name: selected_plan_name,
            exclude,
        }) => {
            let plan_id = encoded_id(plan_id)?;
            if plan_name.is_empty() {
                plan_name = selected_plan_name.clone().unwrap_or_default();
            }
            let catalog = state.test_catalog.as_ref().ok_or_else(|| {
                DeviceRepositoryError::Internal("TestRail is not configured".to_owned())
            })?;
            let catalog_suites = catalog
                .suites(plan_id as u64)
                .await
                .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
            let excluded_suites = exclude
                .as_ref()
                .map(|exclude| {
                    exclude
                        .suites
                        .iter()
                        .map(|value| encoded_id(value))
                        .collect::<Result<std::collections::BTreeSet<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            for suite in catalog_suites {
                let suite_id = suite.id as i64;
                if excluded_suites.contains(&suite_id) {
                    continue;
                }
                let excluded_cases = exclude
                    .as_ref()
                    .and_then(|exclude| {
                        exclude.cases_by_suite.iter().find_map(|(key, values)| {
                            (encoded_id(key).ok() == Some(suite_id)).then_some(values)
                        })
                    })
                    .map(|values| {
                        values
                            .iter()
                            .map(|value| encoded_id(value))
                            .collect::<Result<std::collections::BTreeSet<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                cases.extend(
                    catalog
                        .cases(plan_id as u64, suite.id, None)
                        .await
                        .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?
                        .into_iter()
                        .filter(|case| !excluded_cases.contains(&(case.id as i64)))
                        .map(|case| catalog_case(case, suite.name.clone())),
                );
            }
            is_all_selected = exclude.as_ref().is_none_or(|exclude| {
                exclude.suites.is_empty() && exclude.cases_by_suite.is_empty()
            });
            suites.clear();
            plan_id
        }
        Some(ExecutionSelection::Partial {
            plan_id,
            plan_name: selected_plan_name,
            suites: selected_suites,
        }) => {
            if selected_suites.is_empty() {
                return Err(DeviceRepositoryError::Validation(
                    "At least one suite selection is required".to_owned(),
                ));
            }
            let plan_id = encoded_id(plan_id)?;
            if plan_name.is_empty() {
                plan_name = selected_plan_name.clone().unwrap_or_default();
            }
            let catalog = state.test_catalog.as_ref().ok_or_else(|| {
                DeviceRepositoryError::Internal("TestRail is not configured".to_owned())
            })?;
            let catalog_suites = catalog
                .suites(plan_id as u64)
                .await
                .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
            suites.clear();
            for selected in selected_suites {
                let suite_id = encoded_id(&selected.suite_id)?;
                if selected.select_all && selected.cases.is_some() {
                    return Err(DeviceRepositoryError::Validation(
                        "Selected cases must be omitted when selectAll is true".to_owned(),
                    ));
                }
                let selected_case_ids = if selected.select_all {
                    None
                } else {
                    let values = selected
                        .cases
                        .as_ref()
                        .filter(|values| !values.is_empty())
                        .ok_or_else(|| {
                            DeviceRepositoryError::Validation(
                                "Selected cases are required when selectAll is false".to_owned(),
                            )
                        })?;
                    Some(
                        values
                            .iter()
                            .map(|value| encoded_id(value))
                            .collect::<Result<std::collections::BTreeSet<_>, _>>()?,
                    )
                };
                let excluded = selected
                    .exclude_cases
                    .as_ref()
                    .map(|values| {
                        values
                            .iter()
                            .map(|value| encoded_id(value))
                            .collect::<Result<std::collections::BTreeSet<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                let suite = catalog_suites
                    .iter()
                    .find(|suite| suite.id as i64 == suite_id)
                    .ok_or_else(|| {
                        DeviceRepositoryError::Validation(format!("Suite {suite_id} not found"))
                    })?;
                suites.push(suite_id);
                cases.extend(
                    catalog
                        .cases(plan_id as u64, suite.id, None)
                        .await
                        .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?
                        .into_iter()
                        .filter(|case| {
                            let case_id = case.id as i64;
                            !excluded.contains(&case_id)
                                && selected_case_ids
                                    .as_ref()
                                    .is_none_or(|ids| ids.contains(&case_id))
                        })
                        .map(|case| catalog_case(case, suite.name.clone())),
                );
            }
            is_all_selected = false;
            plan_id
        }
        None => {
            let plan_id = request.test_plan_id.ok_or_else(|| {
                DeviceRepositoryError::Validation(
                    "Provide selection or a legacy testPlanId and testcase selection".to_owned(),
                )
            })?;
            if !request.is_all_selected && suites.is_empty() && request.test_cases.is_empty() {
                return Err(DeviceRepositoryError::Validation(
                    "Provide selection or a legacy testPlanId and testcase selection".to_owned(),
                ));
            }
            cases.extend(request.test_cases.values().flatten().cloned());
            if request.is_all_selected || !suites.is_empty() {
                let catalog = state.test_catalog.as_ref().ok_or_else(|| {
                    DeviceRepositoryError::Internal("TestRail is not configured".to_owned())
                })?;
                let catalog_suites = catalog
                    .suites(plan_id as u64)
                    .await
                    .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
                for suite in catalog_suites {
                    if request.is_all_selected || suites.contains(&(suite.id as i64)) {
                        cases.extend(
                            catalog
                                .cases(plan_id as u64, suite.id, None)
                                .await
                                .map_err(|error| {
                                    DeviceRepositoryError::Internal(error.to_string())
                                })?
                                .into_iter()
                                .map(|case| catalog_case(case, suite.name.clone())),
                        );
                    }
                }
            }
            plan_id
        }
    };
    let mut unique_cases = std::collections::BTreeMap::new();
    for case in cases {
        unique_cases.entry(case.test_case_id).or_insert(case);
    }
    let mut cases = unique_cases.into_values().collect::<Vec<_>>();
    cases.sort_by_key(|case| (case.suite_id, case.test_case_id));
    if plan_name.is_empty()
        && request
            .name
            .as_deref()
            .is_none_or(|name| name.trim().is_empty())
    {
        if let Some(catalog) = &state.test_catalog {
            plan_name = catalog
                .plans(None)
                .await
                .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?
                .into_iter()
                .find(|plan| plan.id == plan_id as u64)
                .map(|plan| plan.name)
                .unwrap_or_else(|| "Untitled".to_owned());
        } else {
            plan_name = "Untitled".to_owned();
        }
    }
    Ok(ExecutionCreation {
        name: request.name,
        device_family: request.device_family,
        device_type: request.device_type,
        build_id: request.build_id,
        plan_id,
        plan_name,
        test_suites: suites,
        is_all_selected,
        selection,
        cases,
        user_id,
    })
}

fn encoded_id(value: &str) -> Result<i64, super::error::DeviceRepositoryError> {
    let digits = value
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse::<i64>().map_err(|_| {
        super::error::DeviceRepositoryError::Validation(format!("Invalid ID format: {value}"))
    })
}

fn catalog_case(case: crate::test_catalog::TestCase, suite_name: String) -> ExecutionCaseInput {
    ExecutionCaseInput {
        test_case_id: case.id as i64,
        name: None,
        execution_id: None,
        suite_id: case.suite_id as i64,
        script_file: case.script_file,
        suite_name,
        title: case.title,
        plan_id: case.plan_id as i64,
        order: Some(case.order),
        priority_id: Some(case.priority_id as i64),
        result: None,
        pre_condition: case.pre_condition,
        labels: case.labels,
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution", tag = "Tests", summary = "List test executions", description = "Returns current-user executions with bounded pagination, device-type search, allowlisted sorting, and computed duration/result counts.", params(ExecutionListQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated executions", body = ExecutionList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ExecutionListQuery>,
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
    match repository.list_test_executions(&user_id, &query).await {
        Ok(executions) => (response_headers, Json(executions)).into_response(),
        Err(error) => repository_error(error, "Failed to list test executions"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/device/{id}", tag = "Tests", summary = "Get latest execution for a device", description = "Returns the current user's newest execution for the device and attaches configured JSON logs when present.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Complete legacy execution row with logs", body = Object), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Execution not found", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn by_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
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
        .test_execution_by_device(&user_id, &device_id)
        .await
    {
        Ok(Some(mut execution)) => {
            add_logs(&state, &mut execution).await;
            (response_headers, Json(execution)).into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test execution"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/progress/list", tag = "Tests", summary = "List active executions", description = "Returns current-user executions in queued, not-executed, or in-progress states for frontend progress tracking.", security(("bearer_auth" = [])), responses((status = 200, description = "Current-user active executions", body = [ActiveExecutionRecord]), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn progress(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
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
    match repository.in_progress_test_executions(&user_id).await {
        Ok(executions) => (response_headers, Json(executions)).into_response(),
        Err(error) => repository_error(error, "Failed to list test executions"),
    }
}

async fn add_logs(state: &AppState, execution: &mut serde_json::Value) {
    let test_id = execution
        .get("testId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let path =
        std::path::Path::new(&state.config.tests.test_logs_dir).join(format!("{test_id}.json"));
    execution["logs"] = tokio::fs::read(path)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!([]));
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/single/{testId}", tag = "Tests", summary = "Get one execution with cases", description = "Returns one unscoped complete legacy execution row with its ordered testcase rows.", params(("testId" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Complete legacy execution row with testCases", body = Object), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Execution not found", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn single(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.single_test_execution(&test_id).await {
        Ok(Some(execution)) => (response_headers, Json(execution)).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test execution"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/list/{id}", tag = "Tests", summary = "List testcase IDs for an execution", description = "Public device compatibility route returning ordered testcase IDs only while the execution is active and not failed or cancelled.", params(("id" = String, Path)), responses((status = 200, description = "Testcase IDs", body = Vec<String>), (status = 404, description = "Execution not found", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn case_ids(
    State(state): State<Arc<AppState>>,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.execution_case_ids(&test_id).await {
        Ok(Some(ids)) => Json(ids).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test execution"),
    }
}
use crate::reports::{
    ConfluenceUploadRequest, ConfluenceUploadResponse, ReportRequest, ReportStatus,
};
use crate::{auth, error::ErrorResponse, events::ServerEvent, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;

#[utoipa::path(get, path = "/api/v1/device/test/execution/report/{id}", tag = "Tests", summary = "Get execution report metadata", description = "Returns the newest durable execution-report status row for the requested test execution.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Execution report metadata", body = ExecutionReportRecord), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Report not found", body = ErrorResponse), (status = 500, description = "Report query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn report(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.execution_report(&test_id).await {
        Ok(Some(report)) => (response_headers, Json(report)).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "No report found for this execution",
        ),
        Err(error) => repository_error(error, "Failed to get execution report"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/report/{id}/html", tag = "Tests", summary = "Download execution report HTML", description = "Resolves validated device/build/test path segments beneath the configured results root and returns the generated report HTML.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Execution report HTML", body = String, content_type = "text/html"), (status = 400, description = "Invalid report path", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Execution or HTML not found", body = ErrorResponse), (status = 500, description = "Report lookup or file read failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn report_html(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let target = match repository.execution_report_target(&test_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Test execution not found",
            );
        }
        Err(error) => return repository_error(error, "Failed to get execution report HTML"),
    };
    if !safe_segment(&target.0) || !safe_segment(&target.1) || !safe_segment(&test_id) {
        return error_response(StatusCode::BAD_REQUEST, "validation", "Invalid report path");
    }
    let path = std::path::Path::new(&state.config.reports.test_results_dir)
        .join(target.0)
        .join(target.1)
        .join(&test_id)
        .join("report.html");
    match tokio::fs::read_to_string(path).await {
        Ok(html) => {
            let mut response = (response_headers, html).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            response
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Report HTML not found. Generate the report first.",
        ),
        Err(error) => {
            tracing::error!(%error, "failed to read report HTML");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Failed to get execution report HTML",
            )
        }
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/testcase/{testCaseId}/log", tag = "Tests", summary = "Download a testcase log", description = "Canonicalizes the stored testcase output path beneath the configured results root and serves it with a sanitized attachment filename.", params(("testCaseId" = i64, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Testcase log", body = String, content_type = "text/plain", headers(("Content-Disposition" = String, description = "Attachment filename derived from the stored log"))), (status = 400, description = "Stored log path escapes the configured root", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Log not found", body = ErrorResponse), (status = 500, description = "Log lookup or file read failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn testcase_log(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(case_id): Path<i64>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    let stored = match repository.test_case_log_path(case_id).await {
        Ok(Some(path)) => path,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "No log file associated with this test case",
            );
        }
        Err(error) => return repository_error(error, "Failed to download test case log"),
    };
    let root = std::path::Path::new(&state.config.reports.test_results_dir);
    let candidate = std::path::Path::new(&stored);
    let path = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let Ok(root) = tokio::fs::canonicalize(root).await else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "Log file not found");
    };
    let Ok(path) = tokio::fs::canonicalize(path).await else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "Log file not found");
    };
    if !path.starts_with(&root) {
        return error_response(StatusCode::BAD_REQUEST, "validation", "Invalid log path");
    }
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => {
            let mut response = (response_headers, content).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            );
            if let Some(name) = path.file_name().and_then(|name| name.to_str())
                && let Ok(value) =
                    HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
            {
                response
                    .headers_mut()
                    .insert(header::CONTENT_DISPOSITION, value);
            }
            response
        }
        Err(error) => {
            tracing::error!(%error, "failed to read testcase log");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Failed to download test case log",
            )
        }
    }
}

fn safe_segment(value: &str) -> bool {
    let mut components = std::path::Path::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

#[utoipa::path(put, path = "/api/v1/device/test/execution/report/{id}", tag = "Tests", summary = "Queue execution report generation", description = "Creates durable report metadata before enqueueing the bounded native report worker.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Report generation queued", body = ExecutionReportRecord), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Execution missing or incomplete", body = ErrorResponse), (status = 409, description = "Report already active", body = ErrorResponse), (status = 500, description = "Report persistence failed", body = ErrorResponse), (status = 503, description = "Device persistence or report queue unavailable", body = ErrorResponse)))]
pub async fn create_report(
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
    let Some(reports) = &state.reports else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            "Reports are unavailable",
        );
    };
    let target = match repository.report_generation_target(&test_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Test Execution Not Found or not completed yet",
            );
        }
        Err(error) => return repository_error(error, "Failed to create execution report"),
    };
    let job_id = uuid::Uuid::new_v4();
    let report = match repository
        .create_execution_report_record(job_id, &test_id, &target.0, &target.1, &user_id)
        .await
    {
        Ok(report) => report,
        Err(error) => return repository_error(error, "Failed to create execution report"),
    };
    if let Err(error) = reports
        .enqueue_with_id(
            job_id,
            ReportRequest {
                test_id: test_id.clone(),
                build_version: target.1.clone(),
                device_type: target.0.clone(),
                previous_test_id: None,
                previous_build_version: None,
                created_by: Some(user_id.clone()),
            },
        )
        .await
    {
        tracing::error!(%error, "failed to queue execution report");
        let message = error.to_string();
        let _ = repository
            .update_execution_report_status(job_id, "failed", Some(&message))
            .await;
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            "Report queue is unavailable",
        );
    }
    (response_headers, Json(report)).into_response()
}

#[utoipa::path(put, path = "/api/v1/device/test/execution/report/{id}/upload", tag = "Tests", summary = "Upload an execution report to Confluence", description = "Uploads a completed native report and persists uploaded or failed status before publishing room events.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 202, description = "Report uploaded to Confluence", body = ConfluenceUploadResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Completed report missing", body = ErrorResponse), (status = 500, description = "Report persistence failed", body = ErrorResponse), (status = 502, description = "Confluence upload failed", body = ErrorResponse), (status = 503, description = "Device persistence or reports unavailable", body = ErrorResponse)))]
pub async fn upload_report(
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
    let Some(reports) = &state.reports else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_unavailable",
            "Reports are unavailable",
        );
    };
    let report = match repository.execution_report(&test_id).await {
        Ok(Some(report)) => report,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "No completed report found for this test execution. Generate the report first.",
            );
        }
        Err(error) => return repository_error(error, "Failed to upload execution report"),
    };
    let Some(report_id) = report
        .get("id")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
    else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Invalid execution report record",
        );
    };
    let completed = reports
        .status(report_id)
        .await
        .is_some_and(|job| job.status == ReportStatus::Completed)
        || matches!(
            report.get("status").and_then(serde_json::Value::as_str),
            Some("completed" | "uploaded")
        );
    if !completed {
        return error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "No completed report found for this test execution. Generate the report first.",
        );
    }
    let device_type = report
        .get("deviceType")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let build_version = report
        .get("buildVersion")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if let Err(error) = repository
        .update_execution_report_status(report_id, "uploading", None)
        .await
    {
        return repository_error(error, "Failed to update execution report");
    }
    publish_report_update(
        &state,
        report_id,
        &test_id,
        &user_id,
        "uploading",
        None,
        None,
    );
    match reports
        .upload_confluence(ConfluenceUploadRequest {
            test_id: test_id.clone(),
            build_version: build_version.to_owned(),
            device_type: device_type.to_owned(),
        })
        .await
    {
        Ok(result) => {
            if let Err(error) = repository
                .update_execution_report_status(report_id, "uploaded", None)
                .await
            {
                return repository_error(error, "Failed to update execution report");
            }
            publish_report_update(
                &state, report_id, &test_id, &user_id, "uploaded", None, None,
            );
            (StatusCode::ACCEPTED, response_headers, Json(result)).into_response()
        }
        Err(error) => {
            let message = error.to_string();
            if let Err(status_error) = repository
                .update_execution_report_status(report_id, "failed", Some(&message))
                .await
            {
                tracing::error!(%status_error, %report_id, "failed to persist report upload failure");
            } else {
                publish_report_update(
                    &state,
                    report_id,
                    &test_id,
                    &user_id,
                    "failed",
                    Some(&format!("Confluence upload failed: {message}")),
                    Some(&message),
                );
            }
            tracing::error!(%error, "Confluence upload failed");
            error_response(
                StatusCode::BAD_GATEWAY,
                "confluence_error",
                "Failed to upload execution report",
            )
        }
    }
}

fn publish_report_update(
    state: &AppState,
    report_id: uuid::Uuid,
    test_id: &str,
    user_id: &str,
    status: &str,
    error: Option<&str>,
    upload_error: Option<&str>,
) {
    for event in report_update_events(report_id, test_id, user_id, status, error, upload_error) {
        if let Err(publish_error) = state.event_publisher.publish(event) {
            tracing::warn!(%publish_error, %report_id, "failed to publish report update");
        }
    }
}

pub(super) fn report_update_events(
    report_id: uuid::Uuid,
    test_id: &str,
    user_id: &str,
    status: &str,
    error: Option<&str>,
    upload_error: Option<&str>,
) -> [ServerEvent; 2] {
    let mut payload = serde_json::json!({
        "reportId": report_id,
        "testExecutionId": test_id,
        "status": status,
        "createdBy": user_id,
    });
    if let Some(error) = error {
        payload["error"] = error.into();
    }
    if let Some(upload_error) = upload_error {
        payload["uploadError"] = upload_error.into();
    }
    [format!("user:{user_id}"), format!("test:{test_id}")].map(|room| ServerEvent {
        event: "execution_report_update".to_owned(),
        payload: payload.clone(),
        room: Some(room),
    })
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/cases/{id}", tag = "Tests", summary = "List execution testcase results", description = "Returns ordered testcase result projections only when the execution belongs to the current user; missing ownership preserves the legacy 500.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Execution testcases", body = [ExecutionCaseRecord]), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Execution missing or lookup failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn cases(
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
    match repository.execution_cases(&user_id, &test_id).await {
        Ok(Some(cases)) => (response_headers, Json(cases)).into_response(),
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Failed to get test cases for execution: Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test cases for execution"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/testcase/{testCaseId}", tag = "Tests", summary = "Get testcase detail", description = "Returns the complete unscoped legacy testcase row; a missing row intentionally preserves the historical 500 response.", params(("testCaseId" = i64, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Complete legacy testcase row", body = Object), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Testcase missing or lookup failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn testcase(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(case_id): Path<i64>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.test_case(case_id).await {
        Ok(Some(case)) => (response_headers, Json(case)).into_response(),
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Failed to get test case: Test Case Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test case"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/logs/{id}", tag = "Tests", summary = "Download execution logs", description = "Reads configured JSON log entries and returns newline-delimited text; missing or invalid files intentionally produce an empty attachment.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Newline-delimited test logs; missing log files produce an empty download", body = String, content_type = "text/plain", headers(("Content-Disposition" = String, description = "Attachment filename test-{id}-logs.txt"))), (status = 401, description = "Authentication required", body = ErrorResponse)))]
pub async fn logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let path =
        std::path::Path::new(&state.config.tests.test_logs_dir).join(format!("{test_id}.json"));
    let logs = tokio::fs::read(path)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<serde_json::Value>>(&bytes).ok())
        .unwrap_or_default();
    let text = logs
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut response = (response_headers, text).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    if let Ok(value) =
        HeaderValue::from_str(&format!("attachment; filename=\"test-{test_id}-logs.txt\""))
    {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct ExecutionDetailQuery {
    pub table: Option<String>,
}

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct BuildExecutionQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/build/{buildId}", tag = "Tests", summary = "List executions for a build", description = "Returns bounded execution summaries for one build using legacy limit/offset pagination.", params(("buildId" = String, Path), BuildExecutionQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Paginated build executions", body = BuildExecutionList), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn by_build(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(build_id): Path<String>,
    Query(query): Query<BuildExecutionQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let limit = query.limit.unwrap_or(5).max(1);
    let offset = query.offset.unwrap_or(0).max(0);
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository
        .executions_by_build(&build_id, limit, offset)
        .await
    {
        Ok(executions) => (response_headers, Json(executions)).into_response(),
        Err(error) => repository_error(error, "Failed to get test executions by build"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/execution/results/{id}", tag = "Tests", summary = "Get execution results", description = "Returns execution status together with complete ordered legacy testcase rows for result inspection.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Execution status and complete legacy testcase rows", body = Object), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Execution not found", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn results(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(test_id): Path<String>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = &state.device_repository else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_unavailable",
            "Device persistence is unavailable",
        );
    };
    match repository.test_results(&test_id).await {
        Ok(Some(results)) => (response_headers, Json(results)).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test results"),
    }
}

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
                    send_device_cancellation(&state, device_ip, &test_id).await;
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

async fn send_device_cancellation(state: &AppState, device_ip: &str, test_id: &str) {
    let Ok(url) = reqwest::Url::parse(&format!("http://{device_ip}:8888/cancel")) else {
        tracing::warn!(
            device_ip,
            test_id,
            "invalid device address for cancellation"
        );
        return;
    };
    let client = match crate::external_http::client(std::time::Duration::from_secs(5)) {
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
    let _ = state;
}

async fn cleanup_cancelled_preparation(state: &AppState, device_type: &str, test_id: &str) {
    if !safe_segment(device_type) || !safe_segment(test_id) {
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
    cancellation: &super::repository_types::DeferredCancellation,
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

#[utoipa::path(get, path = "/api/v1/device/test/execution/{id}", tag = "Tests", summary = "Get a test execution", description = "Returns current-user detail for an ID or latest active execution; table mode preserves the historically unscoped summary and attaches configured logs.", params(("id" = String, Path), ExecutionDetailQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Complete legacy execution row or table summary with logs", body = Object), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 404, description = "Test execution not found", body = ErrorResponse), (status = 500, description = "Execution query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub async fn execution(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<ExecutionDetailQuery>,
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
    let result = if query.table.is_some() {
        repository.test_execution_summary(&user_id, &id).await
    } else {
        repository.test_execution(&user_id, &id).await
    };
    match result {
        Ok(Some(mut execution)) => {
            add_logs(&state, &mut execution).await;
            (response_headers, Json(execution)).into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Test Execution Not Found",
        ),
        Err(error) => repository_error(error, "Failed to get test execution"),
    }
}
