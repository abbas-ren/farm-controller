use super::*;

#[utoipa::path(get, path = "/api/v1/device/faulty/report", tag = "Faulty Reports", summary = "List faulty reports", description = "Returns all retained faulty reports in descending creation order for administrator review.", security(("bearer_auth" = [])), responses((status = 200, description = "Faulty reports", body = [FaultyReportRecord]), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 500, description = "Report query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let (response_headers, repository) = match context(&state, &headers, Some("admin")).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    match repository.faulty_reports().await {
        Ok(reports) => (response_headers, Json(reports)).into_response(),
        Err(error) => repository_error(error, "listing faulty reports"),
    }
}

async fn report_detail(
    state: &Arc<AppState>,
    repository: &Arc<dyn DeviceRepository>,
    id: Uuid,
) -> Result<serde_json::Value, Box<axum::response::Response>> {
    let Some(mut detail) = repository
        .faulty_report_by_id(id)
        .await
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

#[utoipa::path(get, path = "/api/v1/device/faulty/report/{id}", tag = "Faulty Reports", summary = "Get faulty report details", description = "Returns the report and build version enriched with the current Keycloak reporter projection.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Faulty report details", body = FaultyReportDetail), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report or identity user not found", body = CompatibilityErrorResponse), (status = 500, description = "Report or identity query failed", body = ErrorResponse), (status = 503, description = "Device persistence or identity provider unavailable", body = ErrorResponse)))]
pub(crate) async fn by_id(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> axum::response::Response {
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

#[utoipa::path(patch, path = "/api/v1/device/faulty/report/{id}/status", tag = "Faulty Reports", summary = "Approve or reject a faulty report", description = "Accepts only approved or rejected and atomically updates the report; approval also marks the associated release faulty.", params(("id" = String, Path)), request_body = FaultyReportStatusRequest, security(("bearer_auth" = [])), responses((status = 200, description = "Updated faulty report", body = FaultyReportRecord), (status = 400, description = "Invalid status", body = MessageResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report not found", body = MessageResponse), (status = 500, description = "Report update failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn update_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<FaultyReportStatusRequest>,
) -> axum::response::Response {
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
        Err(error) => repository_error(error, "updating faulty report status"),
    }
}

#[utoipa::path(delete, path = "/api/v1/device/faulty/report/{id}", tag = "Faulty Reports", summary = "Delete a faulty report", description = "Hard-deletes the retained report row for compatibility; stored attachment and log files are intentionally left unchanged.", params(("id" = String, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Faulty report deleted", body = SuccessResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 403, description = "Administrator role required", body = ErrorResponse), (status = 404, description = "Report not found", body = MessageResponse), (status = 500, description = "Report deletion failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> axum::response::Response {
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
        Err(error) => repository_error(error, "deleting faulty report"),
    }
}
