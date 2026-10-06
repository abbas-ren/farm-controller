//! Administrator and public managed-user lookup endpoints.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::auth::{
    ActiveUsersResponse, AuthErrorResponse, AuthMessageResponse, AuthUserResponse, IdentityError,
    ManagedUser, ManagedUsersResponse, errors::legacy_error, session::authorize_request,
};
use crate::state::AppState;

async fn authorized_managed_users(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(HeaderMap, Vec<ManagedUser>), IdentityError> {
    let response_headers = authorize_request(state, headers, Some("admin"))
        .await
        .map_err(|error| IdentityError::upstream(error.status, error.message))?;
    let provider = state.identity_provider.as_ref().ok_or_else(|| {
        IdentityError::upstream(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication is disabled",
        )
    })?;
    let users = provider.managed_users().await?;
    Ok((response_headers, users))
}

#[utoipa::path(get, path = "/api/v1/auth/users", tag = "Authentication", summary = "List approved and pending users", description = "Administrator view that partitions Keycloak users by the configured approved-user realm role.", security(("bearer_auth" = [])), responses((status = 200, description = "Approved and pending users", body = ManagedUsersResponse), (status = 401, description = "Authentication required", body = AuthErrorResponse), (status = 403, description = "Administrator role required", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)))]
pub async fn all_users(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let (response_headers, users) = match authorized_managed_users(&state, &headers).await {
        Ok(result) => result,
        Err(error) => return legacy_error(error.status, &error.message),
    };
    let (approved, requests): (Vec<_>, Vec<_>) = users.into_iter().partition(|user| {
        user.realm_roles
            .iter()
            .any(|role| role == &state.config.auth.user_role)
    });
    (
        StatusCode::OK,
        response_headers,
        Json(serde_json::json!({ "requests": requests, "users": approved })),
    )
        .into_response()
}

#[utoipa::path(get, path = "/api/v1/auth/users/active", tag = "Authentication", summary = "List active approved users", description = "Returns only Keycloak users carrying the configured approved-user realm role.", security(("bearer_auth" = [])), responses((status = 200, description = "Approved users", body = ActiveUsersResponse), (status = 401, description = "Authentication required", body = AuthErrorResponse), (status = 403, description = "Administrator role required", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)))]
pub async fn active_users(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let (response_headers, users) = match authorized_managed_users(&state, &headers).await {
        Ok(result) => result,
        Err(error) => return legacy_error(error.status, &error.message),
    };
    let users: Vec<_> = users
        .into_iter()
        .filter(|user| {
            user.realm_roles
                .iter()
                .any(|role| role == &state.config.auth.user_role)
        })
        .collect();
    (
        StatusCode::OK,
        response_headers,
        Json(serde_json::json!({ "data": users })),
    )
        .into_response()
}

#[utoipa::path(get, path = "/api/v1/auth/user/{user_id}", tag = "Authentication", summary = "Get a managed user", description = "Public legacy lookup used by registration and reset flows; returns the stable frontend user projection rather than the complete Keycloak record.", params(("user_id" = String, Path)), responses((status = 200, description = "User details", body = AuthUserResponse), (status = 404, description = "User not found", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)))]
pub async fn user_by_id(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Response {
    let Some(provider) = &state.identity_provider else {
        return legacy_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication is disabled",
        );
    };
    match provider.user_by_id(&user_id).await {
        Ok(user) => Json(serde_json::json!({ "id": user.id, "userName": user.username, "email": user.email, "firstName": user.first_name, "lastName": user.last_name })).into_response(),
        Err(error) => legacy_error(error.status, &error.message),
    }
}

#[utoipa::path(delete, path = "/api/v1/auth/user/{user_id}", tag = "Authentication", summary = "Delete a managed user", description = "Permanently deletes the Keycloak user after administrator authorization.", security(("bearer_auth" = [])), params(("user_id" = String, Path)), responses((status = 200, description = "User deleted", body = AuthMessageResponse), (status = 401, description = "Authentication required", body = AuthErrorResponse), (status = 403, description = "Administrator role required", body = AuthErrorResponse), (status = 404, description = "User not found", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)))]
pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Response {
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
    match provider.delete_user(&user_id).await {
        Ok(()) => (
            StatusCode::OK,
            response_headers,
            Json(serde_json::json!({ "message": "User deleted successfully!" })),
        )
            .into_response(),
        Err(error) => legacy_error(error.status, &error.message),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/users/requests",
    tag = "Authentication",
    summary = "List pending user registrations",
    description = "Lists Keycloak users that do not yet carry the configured approved-user realm role.",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Pending users", body = ActiveUsersResponse), (status = 401, description = "Invalid session", body = AuthErrorResponse), (status = 403, description = "Admin role required", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse))
)]
pub async fn pending_users(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
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
    match provider.pending_users().await {
        Ok(users) => (
            StatusCode::OK,
            response_headers,
            Json(serde_json::json!({ "data": users })),
        )
            .into_response(),
        Err(error) => legacy_error(error.status, &error.message),
    }
}
