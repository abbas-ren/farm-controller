use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::{
    auth,
    devices::{ControllerListQuery, EDGE_CONTROLLER_PORT},
    observability::{
        RuntimeLogEntry, recent_runtime_logs, runtime_log_level, set_runtime_log_level,
    },
    state::AppState,
};

const DEFAULT_LOG_LIMIT: usize = 500;
const MAX_LOG_LIMIT: usize = 5_000;
const EDGE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeLogSource {
    FarmController,
    EdgeController,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogsQuery {
    source: RuntimeLogSource,
    controller_id: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogLevelRequest {
    source: RuntimeLogSource,
    controller_id: Option<String>,
    level: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeLogsResponse {
    source: RuntimeLogSource,
    level: String,
    logs: Vec<RuntimeLogEntry>,
}

pub async fn logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RuntimeLogsQuery>,
) -> Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LOG_LIMIT)
        .clamp(1, MAX_LOG_LIMIT);
    match query.source {
        RuntimeLogSource::FarmController => {
            let level = match runtime_log_level() {
                Ok(level) => level,
                Err(error) => return internal_error(error.to_string()),
            };
            let logs = match recent_runtime_logs(limit) {
                Ok(logs) => logs,
                Err(error) => return internal_error(error.to_string()),
            };
            (
                response_headers,
                Json(RuntimeLogsResponse {
                    source: RuntimeLogSource::FarmController,
                    level,
                    logs,
                }),
            )
                .into_response()
        }
        RuntimeLogSource::EdgeController => {
            proxy_edge(
                &state,
                response_headers,
                query.controller_id.as_deref(),
                reqwest::Method::GET,
                Some(limit),
                None,
            )
            .await
        }
    }
}

pub async fn update_level(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RuntimeLogLevelRequest>,
) -> Response {
    let response_headers = match auth::authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    match request.source {
        RuntimeLogSource::FarmController => match set_runtime_log_level(&request.level) {
            Ok(level) => {
                tracing::info!(%level, "FarmController runtime log level updated");
                (response_headers, Json(serde_json::json!({"level": level}))).into_response()
            }
            Err(error) => bad_request(error.to_string()),
        },
        RuntimeLogSource::EdgeController => {
            proxy_edge(
                &state,
                response_headers,
                request.controller_id.as_deref(),
                reqwest::Method::PUT,
                None,
                Some(serde_json::json!({"level": request.level})),
            )
            .await
        }
    }
}

async fn proxy_edge(
    state: &AppState,
    response_headers: HeaderMap,
    controller_id: Option<&str>,
    method: reqwest::Method,
    limit: Option<usize>,
    body: Option<serde_json::Value>,
) -> Response {
    let Some(controller_id) = controller_id.filter(|value| !value.trim().is_empty()) else {
        return bad_request("controllerId is required for EdgeController logs");
    };
    let address = match controller_address(state, controller_id).await {
        Ok(Some(address)) => address,
        Ok(None) => {
            return error(
                StatusCode::NOT_FOUND,
                "controller_not_found",
                "Controller not found",
            );
        }
        Err(message) => return internal_error(message),
    };
    let path = if method == reqwest::Method::GET {
        "logs"
    } else {
        "logs/level"
    };
    let mut url =
        match reqwest::Url::parse(&format!("http://{address}:{EDGE_CONTROLLER_PORT}/{path}")) {
            Ok(url) => url,
            Err(_) => {
                return error(
                    StatusCode::BAD_GATEWAY,
                    "controller_address",
                    "Invalid EdgeController address",
                );
            }
        };
    if let Some(limit) = limit {
        url.query_pairs_mut()
            .append_pair("limit", &limit.to_string());
    }
    let client = match reqwest::Client::builder()
        .timeout(EDGE_REQUEST_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return internal_error(format!("cannot create EdgeController client: {error}"));
        }
    };
    let mut request = client.request(method, url);
    if let Ok(token) = std::env::var("EDGE_CONTROLLER_TOKEN")
        && !token.trim().is_empty()
    {
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let edge_response = match request.send().await {
        Ok(response) => response,
        Err(request_error) => {
            tracing::warn!(error = %request_error, controller_id, "EdgeController logging request failed");
            return error(
                StatusCode::BAD_GATEWAY,
                "edge_unavailable",
                "EdgeController logging request failed",
            );
        }
    };
    let status: axum::http::StatusCode =
        axum::http::StatusCode::from_u16(edge_response.status().as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY);
    let payload = edge_response
        .json::<serde_json::Value>()
        .await
        .unwrap_or_else(
            |_| serde_json::json!({"error": "EdgeController returned an invalid response"}),
        );
    (status, response_headers, Json(payload)).into_response()
}

async fn controller_address(
    state: &AppState,
    controller_id: &str,
) -> Result<Option<String>, String> {
    let repository = state
        .device_repository
        .as_ref()
        .ok_or_else(|| "Device persistence is unavailable".to_owned())?;
    let controllers = repository
        .list_controllers(&ControllerListQuery {
            search: Some(controller_id.to_owned()),
            status: None,
            state: None,
            sort_by: None,
            desc: None,
            page: Some(1),
            limit: Some(100),
        })
        .await
        .map_err(|error| format!("Failed to load controller: {error}"))?;
    Ok(controllers.data.into_iter().find_map(|controller| {
        let id = controller.get("deviceControllerId")?.as_str()?;
        (id == controller_id)
            .then(|| controller.get("ipAddress")?.as_str().map(str::to_owned))
            .flatten()
    }))
}

fn bad_request(message: impl Into<String>) -> Response {
    error(StatusCode::BAD_REQUEST, "invalid_request", message)
}

fn internal_error(message: impl Into<String>) -> Response {
    tracing::error!(error = %message.into(), "runtime logging request failed");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "logging_unavailable",
        "Runtime logging is unavailable",
    )
}

fn error(status: StatusCode, code: &'static str, message: impl Into<String>) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": {"code": code, "message": message.into()}
        })),
    )
        .into_response()
}
