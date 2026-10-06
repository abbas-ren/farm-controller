//! Authentication request headers, cookies, token subjects, and authorization.

use axum::http::{HeaderMap, HeaderValue, StatusCode, header::SET_COOKIE};

use crate::auth::AuthorizationError;
use crate::auth::constants::SESSION_EXPIRED;
use crate::state::AppState;

/// Validates the incoming access token and returns cookies for any refreshed session.
pub async fn authorize_request(
    state: &AppState,
    headers: &HeaderMap,
    required_role: Option<&str>,
) -> Result<HeaderMap, AuthorizationError> {
    let Some(access_token) = bearer_header(headers, axum::http::header::AUTHORIZATION)
        .or_else(|| cookie_value(headers, "accessToken"))
    else {
        return Err(AuthorizationError {
            status: StatusCode::UNAUTHORIZED,
            message: SESSION_EXPIRED.to_owned(),
        });
    };
    let refresh_token = refresh_token(headers);
    let Some(provider) = &state.identity_provider else {
        return Err(AuthorizationError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "Authentication is disabled".to_owned(),
        });
    };

    match provider
        .validate(&access_token, refresh_token.as_deref(), required_role)
        .await
    {
        Ok(result) => {
            let mut response_headers = HeaderMap::new();
            if let Some(tokens) = result.refreshed_tokens {
                append_cookie(
                    &mut response_headers,
                    "accessToken",
                    &tokens.access_token,
                    tokens.expires_in,
                    state.config.auth.secure_cookies,
                );
                append_cookie(
                    &mut response_headers,
                    "refreshToken",
                    &tokens.refresh_token,
                    7 * 24 * 60 * 60,
                    state.config.auth.secure_cookies,
                );
            }
            Ok(response_headers)
        }
        Err(error) => {
            tracing::warn!(
                status = %error.status,
                required_role = ?required_role,
                "identity provider rejected authorization"
            );
            Err(AuthorizationError {
                status: error.status,
                message: error.message,
            })
        }
    }
}

/// Extracts the subject claim from a bearer token or access-token cookie.
pub(crate) fn token_subject(headers: &HeaderMap) -> Option<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    let token = bearer_header(headers, axum::http::header::AUTHORIZATION)
        .or_else(|| cookie_value(headers, "accessToken"))?;
    let payload = token.split('.').nth(1)?;
    let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice::<serde_json::Value>(&payload)
        .ok()?
        .get("sub")?
        .as_str()
        .map(str::to_owned)
}

fn bearer_header(headers: &HeaderMap, name: axum::http::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .strip_prefix("Bearer ")
                .unwrap_or(value)
                .trim()
                .to_owned()
        })
        .filter(|value| !value.is_empty())
}

fn refresh_token(headers: &HeaderMap) -> Option<String> {
    ["refreshtoken", "x-refresh-token"]
        .into_iter()
        .find_map(|name| bearer_header(headers, axum::http::HeaderName::from_static(name)))
        .or_else(|| cookie_value(headers, "refreshToken"))
}

fn cookie_value(headers: &HeaderMap, expected_name: &str) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == expected_name).then(|| value.to_owned())
            })
        })
        .filter(|value| !value.is_empty())
}

pub(super) fn append_cookie(
    headers: &mut HeaderMap,
    name: &str,
    value: &str,
    max_age: u64,
    secure: bool,
) {
    let secure_attribute = if secure { "; Secure" } else { "" };
    let cookie = format!(
        "{name}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure_attribute}"
    );
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.append(SET_COOKIE, value);
    }
}
