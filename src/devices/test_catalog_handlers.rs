use super::error_response;
use crate::{auth, error::ErrorResponse, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestPlanQuery {
    pub filter: Option<String>,
    pub build_id: Option<String>,
    pub device_id: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestSuiteQuery {
    pub plan_id: Option<u64>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestCaseQuery {
    pub plan_id: Option<u64>,
    pub suite_id: Option<u64>,
    pub filter: Option<String>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[utoipa::path(get, path = "/api/v1/device/test/plan/{planID}/testcase", tag = "Tests", summary = "List Qmetry cases by plan", description = "Loads the Qmetry folder hierarchy and returns provider-defined testcase projections grouped by child suite ID.", params(("planID" = u64, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Provider-defined cases grouped by suite", body = Object), (status = 400, description = "Invalid plan ID", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Qmetry request failed", body = ErrorResponse), (status = 503, description = "Qmetry is not configured", body = ErrorResponse)))]
pub async fn qmetry_plan_cases(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(plan_id): Path<u64>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    if plan_id < 7 {
        return error_response(StatusCode::BAD_REQUEST, "validation", "PlanID is invalid");
    }
    let Some(catalog) = &state.qmetry_catalog else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "qmetry_unavailable",
            "Qmetry is not configured",
        );
    };
    match catalog.cases_by_plan(plan_id).await {
        Ok(cases) => (response_headers, Json(cases)).into_response(),
        Err(error) => {
            tracing::error!(%error, "Qmetry plan case lookup failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "qmetry_error",
                "Failed to fetch test cases",
            )
        }
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/plan/{planID}/suite/{suiteID}/testcase", tag = "Tests", summary = "List Qmetry cases by suite", description = "Searches one Qmetry folder and returns its provider-defined legacy testcase projection; planID is validated for compatibility.", params(("planID" = u64, Path), ("suiteID" = u64, Path)), security(("bearer_auth" = [])), responses((status = 200, description = "Provider-defined suite test cases", body = Object), (status = 400, description = "Invalid suite ID", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Qmetry request failed", body = ErrorResponse), (status = 503, description = "Qmetry is not configured", body = ErrorResponse)))]
pub async fn qmetry_suite_cases(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((plan_id, suite_id)): Path<(u64, u64)>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    if plan_id < 7 || suite_id < 7 {
        return error_response(StatusCode::BAD_REQUEST, "validation", "SuiteID is invalid");
    }
    let Some(catalog) = &state.qmetry_catalog else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "qmetry_unavailable",
            "Qmetry is not configured",
        );
    };
    match catalog.cases_by_folder(suite_id).await {
        Ok(cases) => (response_headers, Json(cases)).into_response(),
        Err(error) => {
            tracing::error!(%error, "Qmetry suite case lookup failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "qmetry_error",
                "Failed to fetch test cases",
            )
        }
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/testcase", tag = "Tests", summary = "List TestRail cases", description = "Fetches all same-origin TestRail pages for the required plan/suite, normalizing script, precondition, labels, and HTML text fields.", params(TestCaseQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Test cases", body = Vec<crate::test_catalog::TestCase>), (status = 400, description = "planId and suiteId are required", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "TestRail request failed", body = ErrorResponse), (status = 503, description = "TestRail is not configured", body = ErrorResponse)))]
pub async fn cases(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TestCaseQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let (Some(plan_id), Some(suite_id)) = (
        query.plan_id.filter(|id| *id > 0),
        query.suite_id.filter(|id| *id > 0),
    ) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "validation",
            "planId and suiteId is required",
        );
    };
    let _ = (query.limit, query.offset);
    let Some(catalog) = &state.test_catalog else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "test_catalog_unavailable",
            "TestRail is not configured",
        );
    };
    match catalog
        .cases(plan_id, suite_id, query.filter.as_deref())
        .await
    {
        Ok(cases) => (response_headers, Json(cases)).into_response(),
        Err(error) => {
            tracing::error!(%error, "TestRail case lookup failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "test_catalog_error",
                "Failed to fetch test cases",
            )
        }
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/suite", tag = "Tests", summary = "List TestRail suites", description = "Fetches all same-origin TestRail section pages for the required plan and returns the stable suite projection.", params(TestSuiteQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Test suites", body = Vec<crate::test_catalog::TestSuite>), (status = 400, description = "planId is required", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "TestRail request failed", body = ErrorResponse), (status = 503, description = "TestRail is not configured", body = ErrorResponse)))]
pub async fn suites(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TestSuiteQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(plan_id) = query.plan_id.filter(|id| *id > 0) else {
        return error_response(StatusCode::BAD_REQUEST, "validation", "planId is required");
    };
    let Some(catalog) = &state.test_catalog else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "test_catalog_unavailable",
            "TestRail is not configured",
        );
    };
    match catalog.suites(plan_id).await {
        Ok(suites) => (response_headers, Json(suites)).into_response(),
        Err(error) => {
            tracing::error!(%error, "TestRail suite lookup failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "test_catalog_error",
                "Failed to fetch test suites",
            )
        }
    }
}

#[utoipa::path(get, path = "/api/v1/device/test/plan", tag = "Tests", summary = "List TestRail plans", description = "Fetches all same-origin TestRail suite pages, applies the optional case-insensitive filter, and returns the stable plan projection.", params(TestPlanQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Test plans", body = Vec<crate::test_catalog::TestPlan>), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "TestRail request failed", body = ErrorResponse), (status = 503, description = "TestRail is not configured", body = ErrorResponse)))]
pub async fn plans(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TestPlanQuery>,
) -> axum::response::Response {
    let response_headers = match auth::authorize_request(&state, &headers, None).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let _ = (&query.build_id, &query.device_id);
    let Some(catalog) = &state.test_catalog else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "test_catalog_unavailable",
            "TestRail is not configured",
        );
    };
    match catalog.plans(query.filter.as_deref()).await {
        Ok(plans) => (response_headers, Json(plans)).into_response(),
        Err(error) => {
            tracing::error!(%error, "TestRail plan lookup failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "test_catalog_error",
                "Failed to fetch test plans",
            )
        }
    }
}
