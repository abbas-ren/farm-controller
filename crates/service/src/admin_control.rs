use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::{Arc, OnceLock, RwLock},
    time::Duration,
};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::{
    auth,
    config::AppConfig,
    devices::{ControllerListQuery, EDGE_CONTROLLER_PORT},
    state::AppState,
};

pub const CONTROL_FILE: &str = "/var/lib/farmcontroller/admin-control.json";
const EDGE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlDocument {
    pub config: AppConfig,
    #[serde(default)]
    pub restart_pending: bool,
    #[serde(default)]
    edge_tokens: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlQuery {
    source: ControlSource,
    controller_id: Option<String>,
    device_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ControlSource {
    FarmController,
    EdgeController,
    EdgeAgent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FarmControlPatch {
    config: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FarmAction {
    Restart,
    DiscardStaged,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FarmActionRequest {
    action: FarmAction,
}

static CONTROL: OnceLock<RwLock<ControlDocument>> = OnceLock::new();

pub fn activate(base: AppConfig) -> Result<AppConfig, crate::error::AppError> {
    let mut document = load(Path::new(CONTROL_FILE))?.unwrap_or(ControlDocument {
        config: base,
        restart_pending: false,
        edge_tokens: BTreeMap::new(),
    });
    document.config.validate()?;
    document.restart_pending = false;
    let active = document.config.clone();
    CONTROL.set(RwLock::new(document)).map_err(|_| {
        crate::error::AppError::Configuration("admin control was already initialized".into())
    })?;
    Ok(active)
}

pub fn edge_token(controller_id: &str) -> Option<String> {
    CONTROL.get().and_then(|control| {
        control
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .edge_tokens
            .get(controller_id)
            .cloned()
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/control",
    tag = "Operations",
    summary = "Read controller administration settings",
    description = "Returns redacted FarmController settings or proxies an approved EdgeController or EdgeAgent through FarmController.",
    params(
        ("source" = String, Query, description = "farmcontroller, edgecontroller, or edgeagent"),
        ("controllerId" = Option<String>, Query, description = "Required for EdgeController"),
        ("deviceId" = Option<String>, Query, description = "Required for EdgeAgent")
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Active and staged controller configuration", body = Object),
        (status = 401, description = "Admin authentication required", body = crate::error::ErrorResponse)
    )
)]
pub async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ControlQuery>,
) -> Response {
    let response_headers = match authorize(&state, &headers).await {
        Ok(headers) => headers,
        Err(response) => return *response,
    };
    match query.source {
        ControlSource::FarmController => {
            (response_headers, Json(farm_snapshot(&state))).into_response()
        }
        ControlSource::EdgeController => {
            proxy_edge(
                &state,
                response_headers,
                query.controller_id.as_deref(),
                Method::GET,
                None,
            )
            .await
        }
        ControlSource::EdgeAgent => {
            proxy_agent(
                &state,
                response_headers,
                query.device_id.as_deref(),
                Method::GET,
                None,
            )
            .await
        }
    }
}

#[utoipa::path(
    patch,
    path = "/api/v1/admin/control",
    tag = "Operations",
    summary = "Update controller administration settings",
    description = "Validates and persists a typed FarmController patch or proxies a validated EdgeController patch. Secret fields are write-only.",
    request_body = Object,
    params(
        ("source" = String, Query, description = "farmcontroller, edgecontroller, or edgeagent"),
        ("controllerId" = Option<String>, Query, description = "Required for EdgeController"),
        ("deviceId" = Option<String>, Query, description = "Required for EdgeAgent")
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Validated configuration updated", body = Object),
        (status = 400, description = "Invalid configuration", body = Object),
        (status = 401, description = "Admin authentication required", body = crate::error::ErrorResponse)
    )
)]
pub async fn patch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ControlQuery>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let response_headers = match authorize(&state, &headers).await {
        Ok(headers) => headers,
        Err(response) => return *response,
    };
    match query.source {
        ControlSource::FarmController => {
            let request = match serde_json::from_value::<FarmControlPatch>(payload) {
                Ok(request) => request,
                Err(error) => return bad_request(format!("invalid FarmController patch: {error}")),
            };
            match patch_farm(&state, request) {
                Ok(snapshot) => (response_headers, Json(snapshot)).into_response(),
                Err(error) => bad_request(error),
            }
        }
        ControlSource::EdgeController => {
            proxy_edge(
                &state,
                response_headers,
                query.controller_id.as_deref(),
                Method::PATCH,
                Some(payload),
            )
            .await
        }
        ControlSource::EdgeAgent => {
            proxy_agent(
                &state,
                response_headers,
                query.device_id.as_deref(),
                Method::PATCH,
                Some(payload),
            )
            .await
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/control",
    tag = "Operations",
    summary = "Run a controller administration action",
    description = "Runs only allowlisted restart, staged-configuration, or EdgeController mapping-maintenance actions.",
    request_body = Object,
    params(
        ("source" = String, Query, description = "farmcontroller, edgecontroller, or edgeagent"),
        ("controllerId" = Option<String>, Query, description = "Required for EdgeController"),
        ("deviceId" = Option<String>, Query, description = "Required for EdgeAgent")
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Allowlisted action completed", body = Object),
        (status = 202, description = "Service restart accepted", body = Object),
        (status = 401, description = "Admin authentication required", body = crate::error::ErrorResponse)
    )
)]
pub async fn action(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ControlQuery>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let response_headers = match authorize(&state, &headers).await {
        Ok(headers) => headers,
        Err(response) => return *response,
    };
    match query.source {
        ControlSource::FarmController => {
            let request = match serde_json::from_value::<FarmActionRequest>(payload) {
                Ok(request) => request,
                Err(error) => {
                    return bad_request(format!("invalid FarmController action: {error}"));
                }
            };
            match request.action {
                FarmAction::Restart => {
                    schedule_restart();
                    (
                        StatusCode::ACCEPTED,
                        response_headers,
                        Json(serde_json::json!({
                            "accepted": true,
                            "action": "restart",
                            "service": "farmcontroller.service"
                        })),
                    )
                        .into_response()
                }
                FarmAction::DiscardStaged => match discard_staged(&state) {
                    Ok(snapshot) => (response_headers, Json(snapshot)).into_response(),
                    Err(error) => internal_error(error),
                },
            }
        }
        ControlSource::EdgeController => {
            proxy_edge(
                &state,
                response_headers,
                query.controller_id.as_deref(),
                Method::POST,
                Some(payload),
            )
            .await
        }
        ControlSource::EdgeAgent => {
            proxy_agent(
                &state,
                response_headers,
                query.device_id.as_deref(),
                Method::POST,
                Some(payload),
            )
            .await
        }
    }
}

async fn authorize(state: &AppState, headers: &HeaderMap) -> Result<HeaderMap, Box<Response>> {
    auth::authorize_request(state, headers, Some("admin"))
        .await
        .map_err(|error| Box::new(error.into_response()))
}

fn patch_farm(state: &AppState, request: FarmControlPatch) -> Result<serde_json::Value, String> {
    let control = ensure_control(state)?;
    let mut document = control
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let previous_level = document.config.logging.level.clone();
    let mut candidate =
        serde_json::to_value(&document.config).map_err(|error| error.to_string())?;
    deep_merge(&mut candidate, request.config);
    let next = serde_json::from_value::<AppConfig>(candidate)
        .map_err(|error| format!("invalid configuration: {error}"))?;
    next.validate().map_err(|error| error.to_string())?;

    if previous_level != next.logging.level {
        crate::observability::set_runtime_log_level(&next.logging.level)
            .map_err(|error| error.to_string())?;
    }

    document.restart_pending =
        serde_json::to_value(&state.config).ok() != serde_json::to_value(&next).ok();
    document.config = next;
    persist(Path::new(CONTROL_FILE), &document).map_err(|error| error.to_string())?;
    *control.write().unwrap_or_else(|error| error.into_inner()) = document;
    tracing::info!("FarmController staged configuration updated by administrator");
    Ok(farm_snapshot(state))
}

fn discard_staged(state: &AppState) -> Result<serde_json::Value, String> {
    let control = ensure_control(state)?;
    let mut document = control
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    document.config = state.config.clone();
    document.restart_pending = false;
    persist(Path::new(CONTROL_FILE), &document).map_err(|error| error.to_string())?;
    *control.write().unwrap_or_else(|error| error.into_inner()) = document;
    Ok(farm_snapshot(state))
}

fn ensure_control(state: &AppState) -> Result<&'static RwLock<ControlDocument>, String> {
    if CONTROL.get().is_none() {
        let _ = CONTROL.set(RwLock::new(ControlDocument {
            config: state.config.clone(),
            restart_pending: false,
            edge_tokens: BTreeMap::new(),
        }));
    }
    CONTROL
        .get()
        .ok_or_else(|| "admin control state is unavailable".to_owned())
}

fn farm_snapshot(state: &AppState) -> serde_json::Value {
    let document = ensure_control(state)
        .ok()
        .map(|control| {
            control
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        })
        .unwrap_or(ControlDocument {
            config: state.config.clone(),
            restart_pending: false,
            edge_tokens: BTreeMap::new(),
        });
    let active = redact_config(&state.config);
    let staged = redact_config(&document.config);
    serde_json::json!({
        "service": "farmcontroller",
        "version": env!("CARGO_PKG_VERSION"),
        "restartPending": document.restart_pending,
        "active": active,
        "staged": staged,
        "secrets": secret_status(&document.config),
        "runtime": {
            "logLevel": crate::observability::runtime_log_level().unwrap_or_else(|_| document.config.logging.level.clone()),
            "databaseConnected": state.database.is_some(),
            "authenticationEnabled": state.identity_provider.is_some(),
            "startedAt": chrono::Utc::now()
                .checked_sub_signed(chrono::Duration::from_std(state.started_at.elapsed()).unwrap_or_default())
                .map(|value| value.to_rfc3339())
        },
        "capabilities": [
            "modules", "server", "authentication", "database", "devicePaths",
            "ssh", "workers", "reports", "integrations", "logging", "restart"
        ],
        "constraints": {
            "restartService": "farmcontroller.service",
            "secretFieldsAreWriteOnly": true
        }
    })
}

fn redact_config(config: &AppConfig) -> serde_json::Value {
    let mut value = serde_json::to_value(config).unwrap_or_default();
    for path in secret_paths() {
        replace_path(&mut value, path, serde_json::Value::Null);
    }
    value
}

fn secret_status(config: &AppConfig) -> serde_json::Value {
    let value = serde_json::to_value(config).unwrap_or_default();
    let entries = secret_paths().iter().map(|path| {
        let configured = get_path(&value, path).is_some_and(|value| match value {
            serde_json::Value::Null => false,
            serde_json::Value::String(value) => !value.is_empty(),
            _ => true,
        });
        (path.to_string(), serde_json::Value::Bool(configured))
    });
    serde_json::Value::Object(entries.collect())
}

fn secret_paths() -> &'static [&'static str] {
    &[
        "auth.client_secret",
        "auth.default_user_password",
        "database.url",
        "device.terminal_password",
        "device.rtos_password",
        "device.gen4_ipl_password",
        "device.gen5_ipl_password",
        "tests.test_rail_api_key",
        "tests.qmetry_api_key",
        "tests.jira_api_token",
        "tests.gitlab_access_token",
        "reports.confluence_pat",
    ]
}

fn replace_path(root: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    let mut parts = path.split('.').peekable();
    let mut current = root;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            if let Some(object) = current.as_object_mut() {
                object.insert(part.to_owned(), value);
            }
            return;
        }
        let Some(next) = current.get_mut(part) else {
            return;
        };
        current = next;
    }
}

fn get_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.')
        .try_fold(root, |value, part| value.get(part))
}

fn deep_merge(target: &mut serde_json::Value, patch: serde_json::Value) {
    match (target, patch) {
        (serde_json::Value::Object(target), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                if let Some(existing) = target.get_mut(&key) {
                    deep_merge(existing, value);
                } else {
                    target.insert(key, value);
                }
            }
        }
        (target, patch) => *target = patch,
    }
}

async fn proxy_edge(
    state: &AppState,
    response_headers: HeaderMap,
    controller_id: Option<&str>,
    method: Method,
    payload: Option<serde_json::Value>,
) -> Response {
    let Some(controller_id) = controller_id.filter(|value| !value.trim().is_empty()) else {
        return bad_request("controllerId is required for EdgeController control");
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
    let url = match reqwest::Url::parse(&format!(
        "http://{address}:{EDGE_CONTROLLER_PORT}/admin/control"
    )) {
        Ok(url) => url,
        Err(_) => {
            return error(
                StatusCode::BAD_GATEWAY,
                "controller_address",
                "Invalid EdgeController address",
            );
        }
    };
    let client = match reqwest::Client::builder().timeout(EDGE_TIMEOUT).build() {
        Ok(client) => client,
        Err(error) => {
            return internal_error(format!("cannot create EdgeController client: {error}"));
        }
    };
    let candidate_token = payload
        .as_ref()
        .and_then(|value| value.get("apiToken"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let mut request = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        url,
    );
    if let Some(token) = edge_token(controller_id).or_else(|| candidate_token.clone()) {
        request = request.bearer_auth(token);
    }
    if let Some(payload) = &payload {
        request = request.json(payload);
    }
    let edge_response = match request.send().await {
        Ok(response) => response,
        Err(request_error) => {
            tracing::warn!(error = %request_error, controller_id, "EdgeController control request failed");
            return error(
                StatusCode::BAD_GATEWAY,
                "edge_unavailable",
                "EdgeController control request failed",
            );
        }
    };
    let status =
        StatusCode::from_u16(edge_response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let body = edge_response
        .json::<serde_json::Value>()
        .await
        .unwrap_or_else(|_| serde_json::json!({"error": "EdgeController returned invalid JSON"}));
    if status.is_success() && method == Method::PATCH {
        update_edge_token(controller_id, payload.as_ref());
    }
    (status, response_headers, Json(body)).into_response()
}

async fn proxy_agent(
    state: &AppState,
    response_headers: HeaderMap,
    device_id: Option<&str>,
    method: Method,
    payload: Option<serde_json::Value>,
) -> Response {
    let device_id = match canonical_device_id(device_id) {
        Ok(device_id) => device_id,
        Err(message) => return bad_request(message),
    };
    let address = match approved_device_address(state, &device_id).await {
        Ok(Some(address)) => address,
        Ok(None) => {
            return error(
                StatusCode::NOT_FOUND,
                "device_not_found",
                "Approved device not found",
            );
        }
        Err(message) => return internal_error(message),
    };
    let url = match reqwest::Url::parse(&format!(
        "http://{address}:{EDGE_CONTROLLER_PORT}/admin/control"
    )) {
        Ok(url) => url,
        Err(_) => {
            return error(
                StatusCode::BAD_GATEWAY,
                "device_address",
                "Invalid EdgeAgent address",
            );
        }
    };
    let client = match reqwest::Client::builder().timeout(EDGE_TIMEOUT).build() {
        Ok(client) => client,
        Err(error) => return internal_error(format!("cannot create EdgeAgent client: {error}")),
    };
    let mut request = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        url,
    );
    if let Some(token) = std::env::var("EDGE_AGENT_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty())
    {
        request = request.bearer_auth(token);
    }
    if let Some(payload) = payload {
        request = request.json(&payload);
    }
    let agent_response = match request.send().await {
        Ok(response) => response,
        Err(request_error) => {
            tracing::warn!(error = %request_error, device_id, "EdgeAgent control request failed");
            return error(
                StatusCode::BAD_GATEWAY,
                "agent_unavailable",
                "EdgeAgent control request failed",
            );
        }
    };
    let status =
        StatusCode::from_u16(agent_response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let body = agent_response
        .json::<serde_json::Value>()
        .await
        .unwrap_or_else(|_| serde_json::json!({"error": "EdgeAgent returned invalid JSON"}));
    (status, response_headers, Json(body)).into_response()
}

fn canonical_device_id(device_id: Option<&str>) -> Result<String, &'static str> {
    let Some(device_id) = device_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err("deviceId is required for EdgeAgent control");
    };
    if device_id.len() != 12 || !device_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("deviceId must be a MAC address without separators");
    }
    Ok(device_id.to_ascii_lowercase())
}

async fn approved_device_address(
    state: &AppState,
    device_id: &str,
) -> Result<Option<std::net::Ipv4Addr>, String> {
    let repository = state
        .device_repository
        .as_ref()
        .ok_or_else(|| "Device persistence is unavailable".to_owned())?;
    let device = repository
        .device_by_id(device_id)
        .await
        .map_err(|error| format!("Failed to load device: {error}"))?;
    let Some(device) = device else {
        return Ok(None);
    };
    if device.get("status").and_then(serde_json::Value::as_str) != Some("approved")
        || device.get("deviceId").and_then(serde_json::Value::as_str) != Some(device_id)
    {
        return Ok(None);
    }
    let address = device
        .get("ipAddress")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Approved device has no IP address".to_owned())?
        .parse::<std::net::Ipv4Addr>()
        .map_err(|_| "Approved device has an invalid IPv4 address".to_owned())?;
    Ok(Some(address))
}

fn update_edge_token(controller_id: &str, payload: Option<&serde_json::Value>) {
    let Some(control) = CONTROL.get() else {
        return;
    };
    let mut document = control.write().unwrap_or_else(|error| error.into_inner());
    if payload
        .and_then(|value| value.get("authEnabled"))
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        document.edge_tokens.remove(controller_id);
    } else if let Some(token) = payload
        .and_then(|value| value.get("apiToken"))
        .and_then(serde_json::Value::as_str)
    {
        document
            .edge_tokens
            .insert(controller_id.to_owned(), token.to_owned());
    }
    if let Err(error) = persist(Path::new(CONTROL_FILE), &document) {
        tracing::error!(%error, controller_id, "cannot persist EdgeController control token");
    }
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
            status: Some("approved".to_owned()),
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

pub fn load(path: &Path) -> Result<Option<ControlDocument>, crate::error::AppError> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(crate::error::AppError::Configuration(error.to_string())),
    };
    if contents.len() > 1024 * 1024 {
        return Err(crate::error::AppError::Configuration(
            "admin control file exceeds 1 MiB".into(),
        ));
    }
    let document = serde_json::from_slice::<ControlDocument>(&contents)
        .map_err(|error| crate::error::AppError::Configuration(error.to_string()))?;
    document.config.validate()?;
    Ok(Some(document))
}

fn persist(path: &Path, document: &ControlDocument) -> Result<(), std::io::Error> {
    use std::os::unix::fs::OpenOptionsExt;

    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "control file has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".admin-control-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, document).map_err(std::io::Error::other)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn schedule_restart() {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(750)).await;
        let process_id = std::process::id().to_string();
        match tokio::process::Command::new("kill")
            .args(["-TERM", &process_id])
            .status()
            .await
        {
            Ok(status) if status.success() => {
                tracing::info!("supervised process restart requested")
            }
            Ok(status) => tracing::error!(%status, "process restart signal was rejected"),
            Err(error) => tracing::error!(%error, "cannot signal process restart"),
        }
    });
}

fn bad_request(message: impl Into<String>) -> Response {
    error(StatusCode::BAD_REQUEST, "invalid_request", message)
}

fn internal_error(message: impl Into<String>) -> Response {
    tracing::error!(error = %message.into(), "admin control request failed");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "control_unavailable",
        "Admin control is unavailable",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli::Cli, config::AppConfig};
    use clap::Parser;

    #[test]
    fn redaction_removes_all_secret_values() {
        let mut config =
            AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap();
        config.device.terminal_password = "target-secret".into();
        config.tests.jira_api_token = "jira-secret".into();
        let redacted = redact_config(&config);
        assert!(redacted["device"]["terminal_password"].is_null());
        assert!(redacted["tests"]["jira_api_token"].is_null());
        assert!(!redacted.to_string().contains("target-secret"));
        assert!(!redacted.to_string().contains("jira-secret"));
    }

    #[test]
    fn deep_merge_preserves_unspecified_configuration() {
        let mut value = serde_json::json!({"logging": {"level": "info", "json": false}});
        deep_merge(
            &mut value,
            serde_json::json!({"logging": {"level": "debug"}}),
        );
        assert_eq!(value["logging"]["level"], "debug");
        assert_eq!(value["logging"]["json"], false);
    }

    #[test]
    fn edge_agent_control_requires_canonical_device_identity() {
        assert_eq!(
            canonical_device_id(Some("AABBCCDDEEFF")).unwrap(),
            "aabbccddeeff"
        );
        assert!(canonical_device_id(Some("aa:bb:cc:dd:ee:ff")).is_err());
        assert!(canonical_device_id(Some("../../admin")).is_err());
        assert!(canonical_device_id(None).is_err());
    }
}
