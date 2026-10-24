//! Faulty report list/get/delete/status handlers.
//!
//! Ports Node list/get/delete/patch-status routes under `/faulty/report` from `faultyRoutes.ts`.

use super::*;

/// PURPOSE: List faulty reports
/// USE: HTTP handler invoked via `devices::routes`.
#[utoipa::path(get, path = "/api/v1/device/faulty/report", tag = "Faulty Reports", summary = "List faulty reports", description = "Returns all retained faulty reports in descending creation order for administrator review.", security(("bearer_auth" = [])), responses((status = 200, description = "Faulty reports", body = [FaultyReportRecord]), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Report query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_handlers::records", "list");
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.faulty_reports().await {
        Ok(reports) => (response_headers, Json(reports)).into_response(),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "listing faulty reports"),
    }
}

/// PURPOSE: List faulty reports
/// USE: HTTP handler invoked via `devices::routes`.
async fn report_detail(
    state: &Arc<AppState>,
    repository: &Arc<dyn DeviceRepository>,
    id: Uuid,
) -> Result<serde_json::Value, Box<axum::response::Response>> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_handlers::records", "report_detail");
    let Some(mut detail) = repository
        .faulty_report_by_id(id)
        .await
        // Map: translate repository failure into the legacy HTTP error envelope.
        .map_err(|error| repository_error(error, "getting faulty report"))?
    else {
        return Err(Box::new(
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"message": format!("Report {id} not found")})),
            )
                .into_response(),
        ));
    };
    let created_by = detail["report"]["createdBy"].as_str().unwrap_or_default();
    let Some(provider) = &state.identity_provider else {
        // Guard: device repository is not wired in this process.
        return Err(Box::new(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "auth_unavailable",
            "Authentication is disabled",
        )));
    };
    let user = provider.user_by_id(created_by).await.map_err(|error| {
        Box::new(error_response(
            error.status,
            "identity_error",
            &error.message,
        ))
    })?;
    detail["user"] = serde_json::json!({
        "id": user.id,
        "userName": user.username,
        "firstName": user.first_name,
        "lastName": user.last_name,
    });
    Ok(detail)
}

/// PURPOSE: Get faulty report details
/// USE: HTTP handler invoked via `devices::routes`.
#[utoipa::path(get, path = "/api/v1/device/faulty/report/{id}", tag = "Faulty Reports", summary = "Get faulty report details", description = "Returns the report and build version enriched with the current Keycloak reporter projection.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Faulty report details", body = FaultyReportDetail), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report or identity user not found", body = CompatibilityErrorResponse), (status = 500, description = "Report or identity query failed", body = ErrorResponse), (status = 503, description = "Device persistence or identity provider unavailable", body = ErrorResponse)))]
pub(crate) async fn by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_handlers::records", "by_id");
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let id = match report_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    match report_detail(&state, &repository, id).await {
        Ok(detail) => (response_headers, Json(detail)).into_response(),
        Err(response) => *response,
    }
}

/// PURPOSE: Approve or reject a faulty report
/// USE: HTTP handler invoked via `devices::routes`.
#[utoipa::path(patch, path = "/api/v1/device/faulty/report/{id}/status", tag = "Faulty Reports", summary = "Approve or reject a faulty report", description = "Accepts only approved or rejected and atomically updates the report; approval also marks the associated release faulty.", params(("id" = String, Path)), request_body = FaultyReportStatusRequest, security(("bearer_auth" = [])), responses((status = 200, description = "Updated faulty report", body = FaultyReportRecord), (status = 400, description = "Invalid status", body = MessageResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report not found", body = MessageResponse), (status = 500, description = "Report update failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn update_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<FaultyReportStatusRequest>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_handlers::records", "update_status");
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    if !matches!(request.status.as_str(), "approved" | "rejected") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"message": "Invalid status"})),
        )
            .into_response();
    }
    let id = match report_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    match repository
        .update_faulty_report_status(id, &request.status)
        .await
    {
        Ok(Some(report)) => (response_headers, Json(report)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"message": format!("Report {id} not found")})),
        )
            .into_response(),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "updating faulty report status"),
    }
}

/// PURPOSE: Delete a faulty report
/// USE: HTTP handler invoked via `devices::routes`.
#[utoipa::path(delete, path = "/api/v1/device/faulty/report/{id}", tag = "Faulty Reports", summary = "Delete a faulty report", description = "Hard-deletes the retained report row for compatibility; stored attachment and log files are intentionally left unchanged.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Faulty report deleted", body = SuccessResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report not found", body = MessageResponse), (status = 500, description = "Report deletion failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> axum::response::Response {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_handlers::records", "delete");
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let id = match report_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    match repository.delete_faulty_report(id).await {
        Ok(true) => (response_headers, Json(serde_json::json!({"success": true}))).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"message": format!("Report {id} not found")})),
        )
            .into_response(),
        // Map: translate repository failure into the legacy HTTP error envelope.
        Err(error) => repository_error(error, "deleting faulty report"),
    }
}
