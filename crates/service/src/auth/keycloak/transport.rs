//! Keycloak HTTP error mapping and wire-format response types.

use std::collections::HashMap;

use axum::http::StatusCode;
use reqwest::Response;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::auth::IdentityError;

pub(super) fn transport_error(error: reqwest::Error) -> IdentityError {
    tracing::warn!(%error, "Keycloak transport request failed");
    IdentityError::upstream(
        StatusCode::BAD_GATEWAY,
        "Authentication provider is unavailable",
    )
}

pub(super) async fn decode_response<T: DeserializeOwned>(
    response: Response,
) -> Result<T, IdentityError> {
    let status = response.status();
    if status.is_success() {
        return response.json().await.map_err(|error| {
            tracing::warn!(%error, "Keycloak returned an invalid JSON response");
            IdentityError::upstream(
                StatusCode::BAD_GATEWAY,
                "Authentication provider returned an invalid response",
            )
        });
    }
    let body = response.json::<KeycloakError>().await.unwrap_or_default();
    let message = body
        .error_description
        .or(body.error_message)
        .or(body.error)
        .unwrap_or_else(|| "Authentication request failed".to_owned());
    Err(IdentityError::upstream(status, message))
}

pub(super) async fn decode_empty_response(response: Response) -> Result<(), IdentityError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.json::<KeycloakError>().await.unwrap_or_default();
    let message = body
        .error_description
        .or(body.error_message)
        .or(body.error)
        .unwrap_or_else(|| "Authentication request failed".to_owned());
    Err(IdentityError::upstream(status, message))
}

#[derive(Debug, Deserialize)]
pub(super) struct ClientTokenResponse {
    pub(super) access_token: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TokenResponse {
    pub(super) access_token: String,
    pub(super) refresh_token: String,
    pub(super) expires_in: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct RoleAccess {
    #[serde(default)]
    pub(super) roles: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct IntrospectionResponse {
    pub(super) active: bool,
    pub(super) realm_access: Option<RoleAccess>,
    #[serde(default)]
    pub(super) resource_access: HashMap<String, RoleAccess>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UserInfo {
    pub(super) sub: String,
    pub(super) preferred_username: String,
    pub(super) email: String,
    pub(super) given_name: Option<String>,
    pub(super) family_name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct KeycloakRole {
    pub(super) id: Option<String>,
    pub(super) name: String,
    pub(super) description: Option<String>,
    pub(super) composite: Option<bool>,
    #[serde(rename = "clientRole")]
    pub(super) client_role: Option<bool>,
    #[serde(rename = "containerId")]
    pub(super) container_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KeycloakUser {
    pub(super) id: Option<String>,
    pub(super) username: Option<String>,
    pub(super) email: Option<String>,
    #[serde(rename = "firstName")]
    pub(super) first_name: Option<String>,
    #[serde(rename = "lastName")]
    pub(super) last_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct KeycloakError {
    error: Option<String>,
    error_description: Option<String>,
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}
