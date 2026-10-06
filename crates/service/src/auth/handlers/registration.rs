//! User registration, event webhook, and password-recovery endpoints.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::auth::{
    AdminRegistrationRequest, AdminRegistrationResponse, AuthErrorResponse, AuthMessageResponse,
    ForgotPasswordRequest, RegistrationAction, RegistrationActionRequest, RegistrationRequest,
    ResetPasswordRequest, UserEventRequest, VerifyUserRequest, errors::legacy_error,
    session::authorize_request, validation::validate_registration,
};
use crate::{events::ServerEvent, state::AppState};

#[utoipa::path(post, path = "/api/v1/auth/register", tag = "Authentication", summary = "Create a managed user", description = "Creates a Keycloak user, assigns the requested admin or user role, and returns the legacy registration envelope.", security(("bearer_auth" = [])), request_body = AdminRegistrationRequest, responses((status = 201, description = "User created", body = AdminRegistrationResponse), (status = 400, description = "Invalid user or role", body = AuthErrorResponse), (status = 401, description = "Authentication required", body = AuthErrorResponse), (status = 403, description = "Administrator role required", body = AuthErrorResponse), (status = 409, description = "User already exists", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)))]
pub async fn register_user(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(mut payload): Json<AdminRegistrationRequest>,
) -> Response {
    payload.username = payload.username.trim().to_owned();
    payload.email = payload.email.trim().to_owned();
    payload.role = payload.role.trim().to_ascii_lowercase();
    let registration = RegistrationRequest {
        username: payload.username.clone(),
        email: payload.email.clone(),
        first_name: payload.first_name.clone(),
        last_name: payload.last_name.clone(),
    };
    if let Err(message) = validate_registration(&registration) {
        return legacy_error(StatusCode::BAD_REQUEST, message);
    }
    if !matches!(payload.role.as_str(), "admin" | "user") {
        return legacy_error(StatusCode::BAD_REQUEST, "Role must be admin or user");
    }
    let response_headers = match authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(provider) = &state.identity_provider else {
        return legacy_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication is disabled",
        );
    };
    match provider.register_user(&payload).await {
        Ok(result) => (
            StatusCode::CREATED,
            response_headers,
            Json(serde_json::json!({
                "message": format!("User {} has been successfully added.", payload.username),
                "data": result
            })),
        )
            .into_response(),
        Err(error) => legacy_error(error.status, &error.message),
    }
}

#[utoipa::path(post, path = "/api/v1/auth/user-registration/confirmation/send-mail", tag = "Authentication", summary = "Acknowledge a registration mail event", description = "Compatibility webhook consumed by the Keycloak event-listener provider.", request_body = UserEventRequest, responses((status = 200, description = "Event acknowledged", body = AuthMessageResponse), (status = 400, description = "Invalid event", body = AuthErrorResponse)))]
pub async fn user_event_webhook(Json(payload): Json<UserEventRequest>) -> Response {
    if payload.user_id.trim().is_empty() || payload.event_type.trim().is_empty() {
        return legacy_error(StatusCode::BAD_REQUEST, "userId and eventType are required");
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "message": "An email has been sent to the user." })),
    )
        .into_response()
}

#[utoipa::path(post, path = "/api/v1/auth/forgot-password", tag = "Authentication", summary = "Request a password reset email", description = "Asks Keycloak to send its signed UPDATE_PASSWORD action email; no admin token is embedded by FarmController.", request_body = ForgotPasswordRequest, responses((status = 200, description = "Password reset email requested", body = AuthMessageResponse), (status = 400, description = "Username or email is missing", body = AuthErrorResponse), (status = 404, description = "User not found", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)))]
pub async fn forgot_password(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ForgotPasswordRequest>,
) -> Response {
    let username = payload.email_or_username.trim();
    if username.is_empty() {
        return legacy_error(StatusCode::BAD_REQUEST, "Email or username is required");
    }
    let Some(provider) = &state.identity_provider else {
        return legacy_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication is disabled",
        );
    };
    match provider.forgot_password(username).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "message": "An email has been sent to the user." })),
        )
            .into_response(),
        Err(error) => legacy_error(error.status, &error.message),
    }
}

#[utoipa::path(post, path = "/api/v1/auth/verify-user", tag = "Authentication", summary = "Verify a legacy password-reset link", description = "Retained compatibility endpoint that validates the supplied reset token and Keycloak user before the legacy reset form is shown.", request_body = VerifyUserRequest, responses((status = 200, description = "Reset link and user are valid", body = AuthMessageResponse), (status = 400, description = "Reset link is missing, invalid, or expired", body = AuthErrorResponse), (status = 404, description = "User not found", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)))]
pub async fn verify_user(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<VerifyUserRequest>,
) -> Response {
    let token = payload.token.trim();
    let user_id = payload.user_id.trim();
    if token.is_empty() || user_id.is_empty() {
        return legacy_error(StatusCode::BAD_REQUEST, "The Link has been expired!");
    }
    let Some(provider) = &state.identity_provider else {
        return legacy_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication is disabled",
        );
    };
    match provider.verify_reset_user(token, user_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "message": "User has been successfully validated." })),
        )
            .into_response(),
        Err(error) => legacy_error(error.status, &error.message),
    }
}

#[utoipa::path(post, path = "/api/v1/auth/reset-password", tag = "Authentication", summary = "Reset a user password", description = "Retained compatibility endpoint that validates the legacy reset token and replaces the Keycloak credential. New reset requests use Keycloak-owned signed action emails.", request_body = ResetPasswordRequest, responses((status = 200, description = "Password reset", body = AuthMessageResponse), (status = 400, description = "Invalid request or reset link", body = AuthErrorResponse), (status = 404, description = "User not found", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)))]
pub async fn reset_password(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ResetPasswordRequest>,
) -> Response {
    let token = payload.token.trim();
    let user_id = payload.user_id.trim();
    let password = payload.password.trim();
    if token.is_empty() || user_id.is_empty() || password.is_empty() {
        return legacy_error(
            StatusCode::BAD_REQUEST,
            "Token, userId, and password are required",
        );
    }
    let Some(provider) = &state.identity_provider else {
        return legacy_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication is disabled",
        );
    };
    match provider.reset_password(token, user_id, password).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "message": "Your password has been successfully reset." })),
        )
            .into_response(),
        Err(error) => legacy_error(error.status, &error.message),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/register/request",
    tag = "Authentication",
    summary = "Request user registration",
    description = "Creates a pending Keycloak user after applying the legacy username, email, and name validation rules.",
    request_body = RegistrationRequest,
    responses(
        (status = 201, description = "Registration request submitted", body = AuthMessageResponse),
        (status = 400, description = "Invalid registration details", body = AuthErrorResponse),
        (status = 401, description = "Email address is missing", body = AuthErrorResponse),
        (status = 409, description = "User already exists", body = AuthErrorResponse),
        (status = 502, description = "Keycloak unavailable", body = AuthErrorResponse),
        (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)
    )
)]
pub async fn request_registration(
    State(state): State<Arc<AppState>>,
    Json(mut payload): Json<RegistrationRequest>,
) -> Response {
    payload.username = payload.username.trim().to_owned();
    payload.email = payload.email.trim().to_owned();
    if payload.username.is_empty() {
        return legacy_error(StatusCode::BAD_REQUEST, "Username is required");
    }
    if payload.email.is_empty() {
        return legacy_error(StatusCode::UNAUTHORIZED, "Email address is required");
    }
    if let Err(message) = validate_registration(&payload) {
        return legacy_error(StatusCode::BAD_REQUEST, message);
    }
    let Some(provider) = &state.identity_provider else {
        return legacy_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication is disabled",
        );
    };
    match provider.request_registration(&payload).await {
        Ok(user) => {
            if let Err(error) = state.event_publisher.publish(ServerEvent {
                event: "user".to_owned(),
                payload: serde_json::json!({
                    "message": user,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                }),
                room: None,
            }) {
                tracing::warn!(%error, username = payload.username, "failed to publish pending user");
            }
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "message": "Your registration request has been successfully submitted."
                })),
            )
                .into_response()
        }
        Err(error) => legacy_error(error.status, &error.message),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/register/action",
    tag = "Authentication",
    summary = "Accept or decline a user registration",
    description = "Accepting assigns the configured user role and initial credential; declining removes the pending Keycloak user.",
    security(("bearer_auth" = [])),
    request_body = RegistrationActionRequest,
    responses((status = 200, description = "Registration action completed", body = AuthMessageResponse), (status = 400, description = "User ID is missing", body = AuthErrorResponse), (status = 401, description = "Invalid session", body = AuthErrorResponse), (status = 403, description = "Admin role required", body = AuthErrorResponse), (status = 404, description = "User not found", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse))
)]
pub async fn registration_action(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(mut payload): Json<RegistrationActionRequest>,
) -> Response {
    payload.user_id = payload.user_id.trim().to_owned();
    if payload.user_id.is_empty() {
        return legacy_error(StatusCode::BAD_REQUEST, "UserId is required");
    }
    let response_headers = match authorize_request(&state, &headers, Some("admin")).await {
        Ok(headers) => headers,
        Err(error) => return error.into_response(),
    };
    let Some(provider) = &state.identity_provider else {
        return legacy_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication is disabled",
        );
    };
    match provider.registration_action(&payload).await {
        Ok(()) => {
            let action = match payload.action {
                RegistrationAction::Accept => "Accepted",
                RegistrationAction::Decline => "Declined",
            };
            (
                StatusCode::OK,
                response_headers,
                Json(serde_json::json!({
                    "message": format!("User registration request has been {action}.")
                })),
            )
                .into_response()
        }
        Err(error) => legacy_error(error.status, &error.message),
    }
}
