//! Sign-in and access-token validation HTTP endpoints.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::auth::{
    AuthErrorResponse, SigninRequest, SigninResponse, ValidationResponse,
    constants::SIGNIN_SUCCESS,
    errors::legacy_error,
    session::{append_cookie, authorize_request},
};
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/v1/auth/signin",
    tag = "Authentication",
    summary = "Sign in with Keycloak",
    description = "Exchanges a username or email address and password with Keycloak, loads realm roles, and returns the legacy frontend token response. Also sets HTTP-only access and refresh cookies.",
    request_body(content = SigninRequest, example = json!({"emailOrUsername": "farm.user", "password": "correct horse battery staple"})),
    responses(
        (status = 200, description = "Signed in", body = SigninResponse),
        (status = 400, description = "Missing credentials", body = AuthErrorResponse),
        (status = 401, description = "Invalid credentials", body = AuthErrorResponse),
        (status = 502, description = "Keycloak unavailable", body = AuthErrorResponse),
        (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)
    )
)]
pub async fn signin(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SigninRequest>,
) -> Response {
    let username = payload.email_or_username.trim();
    let password = payload.password.trim();
    if username.is_empty() || password.is_empty() {
        return legacy_error(
            StatusCode::BAD_REQUEST,
            "Username and password are required",
        );
    }

    let Some(provider) = &state.identity_provider else {
        return legacy_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication is disabled",
        );
    };
    match provider.signin(username, password).await {
        Ok(result) => {
            let mut headers = HeaderMap::new();
            append_cookie(
                &mut headers,
                "accessToken",
                &result.access_token,
                15 * 60,
                state.config.auth.secure_cookies,
            );
            append_cookie(
                &mut headers,
                "refreshToken",
                &result.refresh_token,
                7 * 24 * 60 * 60,
                state.config.auth.secure_cookies,
            );
            (
                StatusCode::OK,
                headers,
                Json(SigninResponse {
                    message: SIGNIN_SUCCESS.to_owned(),
                    token: result.access_token,
                    refresh_token: result.refresh_token,
                    user: result.user,
                }),
            )
                .into_response()
        }
        Err(error) => {
            if error.status.is_server_error() {
                tracing::error!(status = %error.status, "Keycloak sign-in failed");
            }
            legacy_error(error.status, &error.message)
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/validate",
    tag = "Authentication",
    summary = "Validate an access token",
    description = "Validates the bearer token with Keycloak. An expired token is refreshed when a refresh token is supplied in a cookie, `refreshtoken`, or `x-refresh-token` header.",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Token is active", body = ValidationResponse), (status = 401, description = "Token is missing, inactive, or cannot be refreshed", body = AuthErrorResponse), (status = 502, description = "Keycloak unavailable", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse))
)]
pub async fn validate(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    validate_headers(state, headers, None, "Authenticated").await
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/validate/admin",
    tag = "Authentication",
    summary = "Validate administrator authorization",
    description = "Validates or refreshes the token and requires the administrator realm or client role.",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Token has the admin role", body = ValidationResponse), (status = 401, description = "Token is invalid", body = AuthErrorResponse), (status = 403, description = "Admin role is absent", body = AuthErrorResponse), (status = 502, description = "Keycloak unavailable", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse))
)]
pub async fn validate_admin(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    validate_headers(state, headers, Some("admin"), "Authorized as Admin").await
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/validate/user",
    tag = "Authentication",
    summary = "Validate user authorization",
    description = "Validates or refreshes the token and requires the configured approved-user realm or client role.",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Token has the user role", body = ValidationResponse), (status = 401, description = "Token is invalid", body = AuthErrorResponse), (status = 403, description = "User role is absent", body = AuthErrorResponse), (status = 502, description = "Keycloak unavailable", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse))
)]
pub async fn validate_user(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    validate_headers(state, headers, Some("user"), "Authorized as User").await
}

async fn validate_headers(
    state: Arc<AppState>,
    headers: HeaderMap,
    required_role: Option<&str>,
    success_message: &'static str,
) -> Response {
    match authorize_request(&state, &headers, required_role).await {
        Ok(response_headers) => (
            StatusCode::OK,
            response_headers,
            Json(serde_json::json!({ "message": success_message, "valid": true })),
        )
            .into_response(),
        Err(error) => error.into_response(),
    }
}
