use std::sync::Arc;

use crate::{
    socket::{
        constants::{BROWSER_SUBPROTOCOL, EDGE_SUBPROTOCOL},
        helpers::{browser_auth_headers, browser_authorization},
        routes::{
            edge_agent::device_session, edge_controller::controller_session, terminal,
            test_result::test_result_session, web_cli,
        },
        types::farm::{BrowserAuthorization, NativeSocketQuery},
    },
    state::AppState,
};
use axum::{
    extract::{Query, State, WebSocketUpgrade},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use reqwest::StatusCode;

#[utoipa::path(
    get,
    path = "/ws",
    tag = "Events",
    summary = "Upgrade a native FarmController WebSocket",
    description = "Selects controller heartbeat, device heartbeat, native test-result, authenticated frontend terminal, or authenticated WebCLI mode from query parameters. Controller clients negotiate the web-cli-protocol subprotocol.",
    params(
        ("deviceControllerId" = Option<String>, Query, description = "EdgeController heartbeat identity"),
        ("controllerId" = Option<String>, Query, description = "Legacy controller identity alias"),
        ("deviceId" = Option<String>, Query, description = "Device identity; pair with testId for native results"),
        ("testId" = Option<String>, Query, description = "Active native test execution identity"),
        ("client" = Option<String>, Query, description = "Authenticated browser mode: frontend or cli"),
        ("userName" = Option<String>, Query, description = "Required display name for cli mode"),
        ("adminTerminal" = Option<bool>, Query, description = "Require the admin role and permit the FarmController host terminal")
    ),
    responses(
        (status = 101, description = "WebSocket protocol upgraded"),
        (status = 400, description = "No valid mode parameters supplied"),
        (status = 401, description = "Browser token or test session is invalid"),
        (status = 503, description = "Persistence required by the selected mode is unavailable")
    )
)]
pub(crate) async fn upgrade(
    State(state): State<Arc<AppState>>,
    Query(query): Query<NativeSocketQuery>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::upgrade", "upgrade");
    let browser_authorization = match browser_authorization(&query) {
        Ok(authorization) => authorization,
        Err(()) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let controller_id = query
        .device_controller_id
        .or(query.controller_id)
        .unwrap_or_default();
    if !controller_id.trim().is_empty() {
        return websocket
            .protocols([EDGE_SUBPROTOCOL])
            .on_upgrade(move |socket| controller_session(state, controller_id, socket));
    }
    let browser_client = browser_authorization != BrowserAuthorization::None;
    let required_role = (browser_authorization == BrowserAuthorization::Admin).then_some("admin");
    let browser_headers = browser_auth_headers(&headers);
    if browser_client
        && crate::auth::authorize_request(&state, &browser_headers, required_role)
            .await
            .is_err()
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let authenticated_user_id = browser_client
        .then(|| crate::auth::token_subject(&browser_headers))
        .flatten();
    if query.client.as_deref() == Some("frontend") {
        let allow_farm_host = browser_authorization == BrowserAuthorization::Admin;
        return websocket
            .protocols([BROWSER_SUBPROTOCOL])
            .on_upgrade(move |socket| terminal::session(state, socket, allow_farm_host));
    }
    if query.client.as_deref() == Some("cli")
        && let (Some(user_id), Some(user_name)) = (authenticated_user_id, query.user_name)
        && !user_id.trim().is_empty()
        && !user_name.trim().is_empty()
    {
        return websocket
            .protocols([BROWSER_SUBPROTOCOL])
            .on_upgrade(move |socket| web_cli::session(state, user_id, user_name, socket));
    }
    if let (Some(test_id), Some(device_id)) = (query.test_id, query.device_id.clone()) {
        let orm = match state.database.as_ref() {
            Some(database) => database.orm(),
            None => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
        return match crate::test_results::validate_session(orm, &test_id, &device_id).await {
            Ok(()) => websocket
                .on_upgrade(move |socket| test_result_session(state, test_id, device_id, socket)),
            Err(error) => {
                tracing::warn!(%error, %test_id, %device_id, "rejected native test result session");
                StatusCode::UNAUTHORIZED.into_response()
            }
        };
    }
    let device_id = query.device_id.unwrap_or_default();
    if !device_id.trim().is_empty() {
        return websocket.on_upgrade(move |socket| device_session(state, device_id, socket));
    }
    (
        StatusCode::BAD_REQUEST,
        "deviceControllerId or deviceId is required",
    )
        .into_response()
}
