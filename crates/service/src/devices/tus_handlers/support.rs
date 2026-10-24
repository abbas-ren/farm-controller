//! Shared TUS header, auth, and offset helpers for create/head/patch handlers.

use std::collections::BTreeMap;

use axum::{
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use base64::{Engine, engine::general_purpose::STANDARD};

use crate::{auth, state::AppState};

pub(super) const TUS_VERSION: &str = "1.0.0";

/// PURPOSE: Helper `authorize` supporting device HTTP handlers (authorize).
/// USE: Authenticated frontend (bearer).
pub(super) async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<HeaderMap, Box<axum::response::Response>> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_handlers::support", "authorize");
    // Auth: require any authenticated bearer token.
    auth::authorize_request(state, headers, None)
        .await
        .map_err(|error| Box::new(error.into_response()))
}

/// PURPOSE: Helper `validate_version` supporting device HTTP handlers (validate version).
/// USE: HTTP handler invoked via `devices::routes`.
pub(super) fn validate_version(headers: &HeaderMap) -> Result<(), StatusCode> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_handlers::support", "validate_version");
    if headers
        .get("tus-resumable")
        .and_then(|value| value.to_str().ok())
        == Some(TUS_VERSION)
    {
        Ok(())
    } else {
        Err(StatusCode::PRECONDITION_FAILED)
    }
}

/// PURPOSE: Helper `integer_header` supporting device HTTP handlers (integer header).
/// USE: HTTP handler invoked via `devices::routes`.
pub(super) fn integer_header(headers: &HeaderMap, name: &'static str) -> Result<u64, StatusCode> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_handlers::support", "integer_header");
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)
}

/// PURPOSE: Helper `parse_metadata` supporting device HTTP handlers (parse metadata).
/// USE: HTTP handler invoked via `devices::routes`.
pub(super) fn parse_metadata(headers: &HeaderMap) -> Result<BTreeMap<String, String>, StatusCode> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_handlers::support", "parse_metadata");
    let Some(value) = headers.get("upload-metadata") else {
        return Ok(BTreeMap::new());
    };
    let value = value.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
    value
        .split(',')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            let entry = entry.trim();

            let (key, encoded) = match entry.split_once(' ') {
                Some((key, encoded)) => (key, encoded),
                None => (entry, ""),
            };

            if key.is_empty() {
                return Err(StatusCode::BAD_REQUEST);
            }

            let decoded = if encoded.is_empty() {
                String::new()
            } else {
                let bytes = STANDARD
                    .decode(encoded)
                    .map_err(|_| StatusCode::BAD_REQUEST)?;
                String::from_utf8(bytes).map_err(|_| StatusCode::BAD_REQUEST)?
            };

            Ok((key.to_owned(), decoded))
        })
        .collect()
}

/// PURPOSE: Helper `protocol_headers` supporting device HTTP handlers (protocol headers).
/// USE: HTTP handler invoked via `devices::routes`.
pub(super) fn protocol_headers(mut headers: HeaderMap) -> HeaderMap {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_handlers::support", "protocol_headers");
    headers.insert("tus-resumable", HeaderValue::from_static(TUS_VERSION));
    headers.insert("tus-version", HeaderValue::from_static(TUS_VERSION));
    headers.insert("tus-extension", HeaderValue::from_static("creation"));
    headers
}

/// PURPOSE: Helper `insert_integer` supporting device HTTP handlers (insert integer).
/// USE: HTTP handler invoked via `devices::routes`.
pub(super) fn insert_integer(headers: &mut HeaderMap, name: &'static str, value: u64) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_handlers::support", "insert_integer");
    if let Ok(value) = HeaderValue::from_str(&value.to_string()) {
        headers.insert(name, value);
    }
}
