use std::collections::BTreeMap;

use axum::{
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use base64::{Engine, engine::general_purpose::STANDARD};

use crate::{auth, state::AppState};

pub(super) const TUS_VERSION: &str = "1.0.0";

pub(super) async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<HeaderMap, Box<axum::response::Response>> {
    auth::authorize_request(state, headers, None)
        .await
        .map_err(|error| Box::new(error.into_response()))
}

pub(super) fn validate_version(headers: &HeaderMap) -> Result<(), StatusCode> {
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

pub(super) fn integer_header(headers: &HeaderMap, name: &'static str) -> Result<u64, StatusCode> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)
}

pub(super) fn parse_metadata(headers: &HeaderMap) -> Result<BTreeMap<String, String>, StatusCode> {
    let Some(value) = headers.get("upload-metadata") else {
        return Ok(BTreeMap::new());
    };
    let value = value.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
    value
        .split(',')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            let (key, encoded) = entry
                .trim()
                .split_once(' ')
                .ok_or(StatusCode::BAD_REQUEST)?;
            if key.is_empty() {
                return Err(StatusCode::BAD_REQUEST);
            }
            let decoded = STANDARD
                .decode(encoded)
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            let decoded = String::from_utf8(decoded).map_err(|_| StatusCode::BAD_REQUEST)?;
            Ok((key.to_owned(), decoded))
        })
        .collect()
}

pub(super) fn protocol_headers(mut headers: HeaderMap) -> HeaderMap {
    headers.insert("tus-resumable", HeaderValue::from_static(TUS_VERSION));
    headers.insert("tus-version", HeaderValue::from_static(TUS_VERSION));
    headers.insert("tus-extension", HeaderValue::from_static("creation"));
    headers
}

pub(super) fn insert_integer(headers: &mut HeaderMap, name: &'static str, value: u64) {
    if let Ok(value) = HeaderValue::from_str(&value.to_string()) {
        headers.insert(name, value);
    }
}
