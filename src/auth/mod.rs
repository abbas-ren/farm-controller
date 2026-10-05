#[cfg(test)]
pub mod tests;

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header::SET_COOKIE},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use reqwest::Url;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use utoipa::ToSchema;

use crate::{config::AuthConfig, error::AppError, events::ServerEvent, state::AppState};

const SIGNIN_SUCCESS: &str = "You have successfully signed in.";
const INVALID_CREDENTIALS: &str = "Invalid username or password";
const SESSION_EXPIRED: &str = "Your session has expired. Please log in again.";
const FORBIDDEN: &str = "You do not have permission to perform this action.";

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SigninRequest {
    /// Keycloak username or email address.
    #[schema(example = "farm.user")]
    pub email_or_username: String,
    /// User password. This value is never logged.
    #[schema(example = "correct horse battery staple")]
    pub password: String,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationRequest {
    #[schema(min_length = 3, max_length = 30, example = "farm.user")]
    pub username: String,
    #[schema(example = "farm.user@example.com")]
    pub email: String,
    #[schema(max_length = 50, example = "Farm")]
    pub first_name: String,
    #[schema(max_length = 50, example = "User")]
    pub last_name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, ToSchema)]
pub enum RegistrationAction {
    Accept,
    Decline,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationActionRequest {
    #[schema(example = "1fbb94ab-7a95-47e9-8e38-62ef07647121")]
    pub user_id: String,
    pub action: RegistrationAction,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedUser {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub realm_roles: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VerifyUserRequest {
    #[schema(example = "signed-reset-token")]
    pub token: String,
    #[schema(example = "1fbb94ab-7a95-47e9-8e38-62ef07647121")]
    pub user_id: String,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResetPasswordRequest {
    #[schema(example = "new example password")]
    pub password: String,
    #[schema(example = "1fbb94ab-7a95-47e9-8e38-62ef07647121")]
    pub user_id: String,
    #[schema(example = "signed-reset-token")]
    pub token: String,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForgotPasswordRequest {
    #[schema(example = "farm.user@example.com")]
    pub email_or_username: String,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminRegistrationRequest {
    #[schema(min_length = 3, max_length = 30, example = "farm.admin")]
    pub username: String,
    #[schema(example = "farm.admin@example.com")]
    pub email: String,
    #[schema(max_length = 50, example = "Farm")]
    pub first_name: String,
    #[schema(max_length = 50, example = "Admin")]
    pub last_name: String,
    #[schema(example = "admin")]
    pub role: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminRegistrationResult {
    pub username: String,
    pub assigned_role: String,
    pub user_data: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserEventRequest {
    #[schema(example = "1fbb94ab-7a95-47e9-8e38-62ef07647121")]
    pub user_id: String,
    #[schema(example = "REGISTER")]
    pub event_type: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticatedUser {
    pub id: String,
    pub username: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    pub realm_roles: Vec<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SigninResponse {
    #[schema(example = "You have successfully signed in.")]
    pub message: String,
    #[schema(example = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.example")]
    pub token: String,
    #[schema(example = "example-refresh-token")]
    pub refresh_token: String,
    pub user: AuthenticatedUser,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AuthMessageResponse {
    pub message: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AuthErrorResponse {
    #[schema(example = false)]
    pub success: bool,
    #[schema(example = "Invalid username or password")]
    pub message: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ManagedUsersResponse {
    pub requests: Vec<ManagedUser>,
    pub users: Vec<ManagedUser>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ActiveUsersResponse {
    pub data: Vec<ManagedUser>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthUserResponse {
    pub id: String,
    pub user_name: String,
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AdminRegistrationResponse {
    pub message: String,
    pub data: AdminRegistrationResult,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ValidationResponse {
    #[schema(example = "Authenticated")]
    pub message: String,
    #[schema(example = true)]
    pub valid: bool,
}

#[derive(Clone, Debug)]
pub struct SigninResult {
    pub access_token: String,
    pub refresh_token: String,
    pub user: AuthenticatedUser,
}

#[derive(Clone, Debug)]
pub struct ValidationResult {
    pub refreshed_tokens: Option<RefreshedTokens>,
}

#[derive(Clone, Debug)]
pub struct RefreshedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

#[derive(Debug)]
pub struct IdentityError {
    pub status: StatusCode,
    pub message: String,
}

pub struct AuthorizationError {
    status: StatusCode,
    message: String,
}

impl IntoResponse for AuthorizationError {
    fn into_response(self) -> Response {
        legacy_error(self.status, &self.message)
    }
}

impl IdentityError {
    fn upstream(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    async fn signin(&self, username: &str, password: &str) -> Result<SigninResult, IdentityError>;
    async fn validate(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        required_role: Option<&str>,
    ) -> Result<ValidationResult, IdentityError>;
    async fn request_registration(
        &self,
        _request: &RegistrationRequest,
    ) -> Result<serde_json::Value, IdentityError> {
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "Registration is unavailable",
        ))
    }
    async fn pending_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
    async fn managed_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
    async fn user_by_id(&self, _user_id: &str) -> Result<ManagedUser, IdentityError> {
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
    async fn delete_user(&self, _user_id: &str) -> Result<(), IdentityError> {
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
    async fn verify_reset_user(&self, _token: &str, _user_id: &str) -> Result<(), IdentityError> {
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "Password reset is unavailable",
        ))
    }
    async fn reset_password(
        &self,
        _token: &str,
        _user_id: &str,
        _password: &str,
    ) -> Result<(), IdentityError> {
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "Password reset is unavailable",
        ))
    }
    async fn forgot_password(&self, _username: &str) -> Result<(), IdentityError> {
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "Password reset is unavailable",
        ))
    }
    async fn register_user(
        &self,
        _request: &AdminRegistrationRequest,
    ) -> Result<AdminRegistrationResult, IdentityError> {
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
    async fn registration_action(
        &self,
        _request: &RegistrationActionRequest,
    ) -> Result<(), IdentityError> {
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
}

pub struct KeycloakIdentityProvider {
    client: reqwest::Client,
    base_url: Url,
    config: AuthConfig,
}

impl KeycloakIdentityProvider {
    pub fn new(config: AuthConfig) -> Result<Self, AppError> {
        let base_url = Url::parse(&config.keycloak_url).map_err(|error| {
            AppError::IdentityProvider(format!("invalid Keycloak URL: {error}"))
        })?;
        let client =
            crate::external_http::client(Duration::from_secs(config.request_timeout_seconds))
                .map_err(|error| AppError::IdentityProvider(error.to_string()))?;
        Ok(Self {
            client,
            base_url,
            config,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url, IdentityError> {
        self.base_url
            .join(path)
            .map_err(|error| IdentityError::upstream(StatusCode::BAD_GATEWAY, error.to_string()))
    }

    fn client_secret(&self) -> Result<&str, IdentityError> {
        self.config.client_secret.as_deref().ok_or_else(|| {
            IdentityError::upstream(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Identity provider is not configured",
            )
        })
    }

    async fn token_request(&self, form: &[(&str, &str)]) -> Result<TokenResponse, IdentityError> {
        let url = self.endpoint(&format!(
            "realms/{}/protocol/openid-connect/token",
            self.config.realm
        ))?;
        let response = self
            .client
            .post(url)
            .form(form)
            .send()
            .await
            .map_err(transport_error)?;
        decode_response(response).await
    }

    async fn client_token(&self) -> Result<String, IdentityError> {
        let response = self
            .token_request(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.config.client_id),
                ("client_secret", self.client_secret()?),
            ])
            .await?;
        Ok(response.access_token)
    }

    async fn password_token(
        &self,
        username: &str,
        password: &str,
    ) -> Result<TokenResponse, IdentityError> {
        self.token_request(&[
            ("client_id", &self.config.client_id),
            ("client_secret", self.client_secret()?),
            ("grant_type", "password"),
            ("username", username),
            ("password", password),
            ("scope", "openid profile email"),
        ])
        .await
    }

    async fn introspect(&self, token: &str) -> Result<IntrospectionResponse, IdentityError> {
        let url = self.endpoint(&format!(
            "realms/{}/protocol/openid-connect/token/introspect",
            self.config.realm
        ))?;
        let response = self
            .client
            .post(url)
            .form(&[
                ("token", token),
                ("token_type_hint", "access_token"),
                ("client_id", &self.config.client_id),
                ("client_secret", self.client_secret()?),
            ])
            .send()
            .await
            .map_err(transport_error)?;
        decode_response(response).await
    }

    async fn refresh(&self, refresh_token: &str) -> Result<RefreshedTokens, IdentityError> {
        let response = self
            .token_request(&[
                ("grant_type", "refresh_token"),
                ("client_id", &self.config.client_id),
                ("client_secret", self.client_secret()?),
                ("refresh_token", refresh_token),
            ])
            .await?;
        Ok(RefreshedTokens {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            expires_in: response.expires_in.unwrap_or(15 * 60),
        })
    }

    async fn get_bearer<T: DeserializeOwned>(
        &self,
        path: &str,
        token: &str,
    ) -> Result<T, IdentityError> {
        self.get_bearer_url(self.endpoint(path)?, token).await
    }

    async fn get_bearer_url<T: DeserializeOwned>(
        &self,
        url: Url,
        token: &str,
    ) -> Result<T, IdentityError> {
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(transport_error)?;
        decode_response(response).await
    }
}

#[async_trait]
impl IdentityProvider for KeycloakIdentityProvider {
    async fn signin(&self, username: &str, password: &str) -> Result<SigninResult, IdentityError> {
        let admin_token = self.client_token().await?;
        let token = match self.password_token(username, password).await {
            Ok(token) => token,
            Err(error) if error.status == StatusCode::BAD_REQUEST => {
                let mut users_url =
                    self.endpoint(&format!("admin/realms/{}/users", self.config.realm))?;
                users_url.query_pairs_mut().append_pair("email", username);
                let users: Vec<KeycloakUser> = self.get_bearer_url(users_url, &admin_token).await?;
                let resolved_username = users
                    .first()
                    .and_then(|user| user.username.as_deref())
                    .ok_or_else(|| {
                        IdentityError::upstream(StatusCode::UNAUTHORIZED, INVALID_CREDENTIALS)
                    })?;
                self.password_token(resolved_username, password)
                    .await
                    .map_err(|_| {
                        IdentityError::upstream(StatusCode::UNAUTHORIZED, INVALID_CREDENTIALS)
                    })?
            }
            Err(error) => return Err(error),
        };

        let user_info: UserInfo = self
            .get_bearer(
                &format!(
                    "realms/{}/protocol/openid-connect/userinfo",
                    self.config.realm
                ),
                &token.access_token,
            )
            .await?;
        let roles: Vec<KeycloakRole> = self
            .get_bearer(
                &format!(
                    "admin/realms/{}/users/{}/role-mappings/realm",
                    self.config.realm, user_info.sub
                ),
                &admin_token,
            )
            .await?;

        Ok(SigninResult {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            user: AuthenticatedUser {
                id: user_info.sub,
                username: user_info.preferred_username,
                email: user_info.email,
                first_name: user_info.given_name,
                last_name: user_info.family_name,
                realm_roles: roles.into_iter().map(|role| role.name).collect(),
            },
        })
    }

    async fn validate(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        required_role: Option<&str>,
    ) -> Result<ValidationResult, IdentityError> {
        let initial = self.introspect(access_token).await?;
        let (claims, refreshed_tokens) = if initial.active {
            (initial, None)
        } else if let Some(refresh_token) = refresh_token {
            let refreshed = self
                .refresh(refresh_token)
                .await
                .map_err(|_| IdentityError::upstream(StatusCode::UNAUTHORIZED, SESSION_EXPIRED))?;
            let claims = self.introspect(&refreshed.access_token).await?;
            if !claims.active {
                return Err(IdentityError::upstream(
                    StatusCode::UNAUTHORIZED,
                    SESSION_EXPIRED,
                ));
            }
            (claims, Some(refreshed))
        } else {
            return Err(IdentityError::upstream(
                StatusCode::UNAUTHORIZED,
                SESSION_EXPIRED,
            ));
        };

        if let Some(role) = required_role {
            let realm_role = claims
                .realm_access
                .as_ref()
                .is_some_and(|access| access.roles.iter().any(|candidate| candidate == role));
            let client_role = claims
                .resource_access
                .get(&self.config.client_id)
                .is_some_and(|access| access.roles.iter().any(|candidate| candidate == role));
            if !realm_role && !client_role {
                return Err(IdentityError::upstream(StatusCode::FORBIDDEN, FORBIDDEN));
            }
        }

        Ok(ValidationResult { refreshed_tokens })
    }

    async fn request_registration(
        &self,
        request: &RegistrationRequest,
    ) -> Result<serde_json::Value, IdentityError> {
        let admin_token = self.client_token().await?;
        let user_id = uuid::Uuid::new_v4().to_string();
        let url = self.endpoint(&format!("admin/realms/{}/users", self.config.realm))?;
        let response = self
            .client
            .post(url)
            .bearer_auth(admin_token)
            .json(&serde_json::json!({
                "id": user_id,
                "username": request.username,
                "email": request.email,
                "firstName": request.first_name,
                "lastName": request.last_name,
                "enabled": true,
                "emailVerified": false,
                "requiredActions": ["UPDATE_PASSWORD"]
            }))
            .send()
            .await
            .map_err(transport_error)?;
        decode_empty_response(response).await?;
        Ok(serde_json::json!({
            "id": user_id,
            "username": request.username,
            "firstName": request.first_name,
            "lastName": request.last_name,
            "email": request.email,
            "emailVerified": false,
            "createdTimestamp": chrono::Utc::now().timestamp_millis(),
            "enabled": true,
            "totp": false,
            "disableableCredentialTypes": [],
            "requiredActions": ["UPDATE_PASSWORD"],
            "notBefore": 0,
            "access": {
                "manageGroupMembership": true,
                "view": true,
                "mapRoles": true,
                "impersonate": true,
                "manage": true
            },
            "realmRoles": []
        }))
    }

    async fn pending_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        Ok(self
            .managed_users()
            .await?
            .into_iter()
            .filter(|user| {
                !user
                    .realm_roles
                    .iter()
                    .any(|role| role == &self.config.user_role)
            })
            .collect())
    }

    async fn managed_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        let token = self.client_token().await?;
        let users: Vec<KeycloakUser> = self
            .get_bearer(&format!("admin/realms/{}/users", self.config.realm), &token)
            .await?;
        let mut managed = Vec::new();
        for user in users {
            let Some(id) = user.id.as_deref() else {
                continue;
            };
            let roles: Vec<KeycloakRole> = self
                .get_bearer(
                    &format!(
                        "admin/realms/{}/users/{id}/role-mappings/realm",
                        self.config.realm
                    ),
                    &token,
                )
                .await?;
            if roles.iter().any(|role| role.name == "admin") {
                continue;
            }
            managed.push(ManagedUser {
                id: id.to_owned(),
                username: user.username.unwrap_or_default(),
                email: user.email,
                first_name: user.first_name,
                last_name: user.last_name,
                realm_roles: roles.into_iter().map(|role| role.name).collect(),
            });
        }
        Ok(managed)
    }

    async fn user_by_id(&self, user_id: &str) -> Result<ManagedUser, IdentityError> {
        let token = self.client_token().await?;
        let user: KeycloakUser = self
            .get_bearer(
                &format!("admin/realms/{}/users/{user_id}", self.config.realm),
                &token,
            )
            .await?;
        Ok(ManagedUser {
            id: user.id.unwrap_or_else(|| user_id.to_owned()),
            username: user.username.unwrap_or_default(),
            email: user.email,
            first_name: user.first_name,
            last_name: user.last_name,
            realm_roles: Vec::new(),
        })
    }

    async fn delete_user(&self, user_id: &str) -> Result<(), IdentityError> {
        let token = self.client_token().await?;
        let response = self
            .client
            .delete(self.endpoint(&format!(
                "admin/realms/{}/users/{user_id}",
                self.config.realm
            ))?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(transport_error)?;
        decode_empty_response(response).await
    }

    async fn verify_reset_user(&self, token: &str, user_id: &str) -> Result<(), IdentityError> {
        let claims = self.introspect(token).await?;
        if !claims.active {
            return Err(IdentityError::upstream(
                StatusCode::BAD_REQUEST,
                "The Link has been expired!",
            ));
        }
        let _: KeycloakUser = self
            .get_bearer(
                &format!("admin/realms/{}/users/{user_id}", self.config.realm),
                token,
            )
            .await?;
        Ok(())
    }

    async fn reset_password(
        &self,
        token: &str,
        user_id: &str,
        password: &str,
    ) -> Result<(), IdentityError> {
        self.verify_reset_user(token, user_id).await?;
        let response = self
            .client
            .put(self.endpoint(&format!(
                "admin/realms/{}/users/{user_id}/reset-password",
                self.config.realm
            ))?)
            .bearer_auth(token)
            .json(&serde_json::json!({ "type": "password", "value": password, "temporary": false }))
            .send()
            .await
            .map_err(transport_error)?;
        decode_empty_response(response).await?;
        let response = self
            .client
            .put(self.endpoint(&format!(
                "admin/realms/{}/users/{user_id}",
                self.config.realm
            ))?)
            .bearer_auth(token)
            .json(&serde_json::json!({ "emailVerified": true }))
            .send()
            .await
            .map_err(transport_error)?;
        decode_empty_response(response).await
    }

    async fn forgot_password(&self, username: &str) -> Result<(), IdentityError> {
        let token = self.client_token().await?;
        let mut users_url = self.endpoint(&format!("admin/realms/{}/users", self.config.realm))?;
        users_url
            .query_pairs_mut()
            .append_pair("username", username);
        let mut users: Vec<KeycloakUser> = self.get_bearer_url(users_url, &token).await?;
        if users.is_empty() {
            let mut users_url =
                self.endpoint(&format!("admin/realms/{}/users", self.config.realm))?;
            users_url.query_pairs_mut().append_pair("email", username);
            users = self.get_bearer_url(users_url, &token).await?;
        }
        let user_id = users
            .first()
            .and_then(|user| user.id.as_deref())
            .ok_or_else(|| {
                IdentityError::upstream(
                    StatusCode::NOT_FOUND,
                    format!("User not found for {username}"),
                )
            })?;
        let mut action_url = self.endpoint(&format!(
            "admin/realms/{}/users/{user_id}/execute-actions-email",
            self.config.realm
        ))?;
        action_url
            .query_pairs_mut()
            .append_pair("client_id", &self.config.client_id);
        let response = self
            .client
            .put(action_url)
            .bearer_auth(token)
            .json(&["UPDATE_PASSWORD"])
            .send()
            .await
            .map_err(transport_error)?;
        decode_empty_response(response).await
    }

    async fn register_user(
        &self,
        request: &AdminRegistrationRequest,
    ) -> Result<AdminRegistrationResult, IdentityError> {
        let token = self.client_token().await?;
        let user_id = uuid::Uuid::new_v4().to_string();
        let response = self
            .client
            .post(self.endpoint(&format!("admin/realms/{}/users", self.config.realm))?)
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "id": user_id,
                "username": request.username,
                "email": request.email,
                "firstName": request.first_name,
                "lastName": request.last_name,
                "enabled": true,
                "emailVerified": false,
                "requiredActions": ["UPDATE_PASSWORD"]
            }))
            .send()
            .await
            .map_err(transport_error)?;
        decode_empty_response(response).await?;

        let role: KeycloakRole = self
            .get_bearer(
                &format!("admin/realms/{}/roles/{}", self.config.realm, request.role),
                &token,
            )
            .await?;
        let response = self
            .client
            .post(self.endpoint(&format!(
                "admin/realms/{}/users/{user_id}/role-mappings/realm",
                self.config.realm
            ))?)
            .bearer_auth(&token)
            .json(&[role])
            .send()
            .await
            .map_err(transport_error)?;
        decode_empty_response(response).await?;

        for (path, body) in [
            (
                format!(
                    "admin/realms/{}/users/{user_id}/reset-password",
                    self.config.realm
                ),
                serde_json::json!({
                    "type": "password",
                    "value": self.config.default_user_password,
                    "temporary": false
                }),
            ),
            (
                format!("admin/realms/{}/users/{user_id}", self.config.realm),
                serde_json::json!({ "emailVerified": true }),
            ),
        ] {
            let response = self
                .client
                .put(self.endpoint(&path)?)
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await
                .map_err(transport_error)?;
            decode_empty_response(response).await?;
        }

        let mut user_data: serde_json::Value = self
            .get_bearer(
                &format!("admin/realms/{}/users/{user_id}", self.config.realm),
                &token,
            )
            .await?;
        user_data["realmRoles"] = serde_json::json!([request.role]);
        Ok(AdminRegistrationResult {
            username: request.username.clone(),
            assigned_role: request.role.clone(),
            user_data,
        })
    }

    async fn registration_action(
        &self,
        request: &RegistrationActionRequest,
    ) -> Result<(), IdentityError> {
        let token = self.client_token().await?;
        match request.action {
            RegistrationAction::Decline => {
                let response = self
                    .client
                    .delete(self.endpoint(&format!(
                        "admin/realms/{}/users/{}",
                        self.config.realm, request.user_id
                    ))?)
                    .bearer_auth(token)
                    .send()
                    .await
                    .map_err(transport_error)?;
                decode_empty_response(response).await
            }
            RegistrationAction::Accept => {
                let role: KeycloakRole = self
                    .get_bearer(
                        &format!(
                            "admin/realms/{}/roles/{}",
                            self.config.realm, self.config.user_role
                        ),
                        &token,
                    )
                    .await?;
                let response = self
                    .client
                    .post(self.endpoint(&format!(
                        "admin/realms/{}/users/{}/role-mappings/realm",
                        self.config.realm, request.user_id
                    ))?)
                    .bearer_auth(&token)
                    .json(&[role])
                    .send()
                    .await
                    .map_err(transport_error)?;
                decode_empty_response(response).await?;
                let response = self
                    .client
                    .put(self.endpoint(&format!(
                        "admin/realms/{}/users/{}/reset-password",
                        self.config.realm, request.user_id
                    ))?)
                    .bearer_auth(token)
                    .json(&serde_json::json!({
                        "type": "password",
                        "value": self.config.default_user_password,
                        "temporary": false
                    }))
                    .send()
                    .await
                    .map_err(transport_error)?;
                decode_empty_response(response).await
            }
        }
    }
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/register/request", post(request_registration))
        .route("/register/action", post(registration_action))
        .route("/users/requests", get(pending_users))
        .route("/users", get(all_users))
        .route("/users/active", get(active_users))
        .route("/user/{user_id}", get(user_by_id).delete(delete_user))
        .route("/verify-user", post(verify_user))
        .route("/reset-password", post(reset_password))
        .route("/forgot-password", post(forgot_password))
        .route("/register", post(register_user))
        .route(
            "/user-registration/confirmation/send-mail",
            post(user_event_webhook),
        )
        .route("/signin", post(signin))
        .route("/validate", get(validate))
        .route("/validate/admin", get(validate_admin))
        .route("/validate/user", get(validate_user))
}

#[utoipa::path(post, path = "/api/v1/auth/register", tag = "Authentication", summary = "Create a managed user", description = "Creates a Keycloak user, assigns the requested admin or user role, and returns the legacy registration envelope.", security(("bearer_auth" = [])), request_body = AdminRegistrationRequest, responses((status = 201, description = "User created", body = AdminRegistrationResponse), (status = 400, description = "Invalid user or role", body = AuthErrorResponse), (status = 401, description = "Authentication required", body = AuthErrorResponse), (status = 403, description = "Administrator role required", body = AuthErrorResponse), (status = 409, description = "User already exists", body = AuthErrorResponse), (status = 502, description = "Keycloak request failed", body = AuthErrorResponse), (status = 503, description = "Authentication is disabled", body = AuthErrorResponse)))]
pub async fn register_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
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
    headers: HeaderMap,
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

fn validate_registration(request: &RegistrationRequest) -> Result<(), &'static str> {
    let valid_username = (3..=30).contains(&request.username.len())
        && request
            .username
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_'));
    if !valid_username {
        return Err(
            "Username must be 3-30 characters and contain only letters, numbers, dots, or underscores",
        );
    }
    if !request.email.contains('@')
        || request.email.starts_with('@')
        || request.email.ends_with('@')
    {
        return Err("Invalid email address");
    }
    for (value, missing, invalid) in [
        (
            request.first_name.as_str(),
            "First name is required",
            "First name must contain only letters, numbers, or underscores",
        ),
        (
            request.last_name.as_str(),
            "Last name is required",
            "Last name must contain only letters, numbers, or underscores",
        ),
    ] {
        if value.is_empty() {
            return Err(missing);
        }
        if value.len() > 50
            || !value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return Err(invalid);
        }
    }
    Ok(())
}

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
                tracing::error!(status = %error.status, error = %error.message, "Keycloak sign-in failed");
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
        Err(error) => Err(AuthorizationError {
            status: error.status,
            message: error.message,
        }),
    }
}

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

fn append_cookie(headers: &mut HeaderMap, name: &str, value: &str, max_age: u64, secure: bool) {
    let secure_attribute = if secure { "; Secure" } else { "" };
    let cookie = format!(
        "{name}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure_attribute}"
    );
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.append(SET_COOKIE, value);
    }
}

fn legacy_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({ "success": false, "message": message })),
    )
        .into_response()
}

fn transport_error(error: reqwest::Error) -> IdentityError {
    tracing::warn!(%error, "Keycloak transport request failed");
    IdentityError::upstream(
        StatusCode::BAD_GATEWAY,
        "Authentication provider is unavailable",
    )
}

async fn decode_response<T: DeserializeOwned>(
    response: reqwest::Response,
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

async fn decode_empty_response(response: reqwest::Response) -> Result<(), IdentityError> {
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
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct RoleAccess {
    #[serde(default)]
    roles: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct IntrospectionResponse {
    active: bool,
    realm_access: Option<RoleAccess>,
    #[serde(default)]
    resource_access: HashMap<String, RoleAccess>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    sub: String,
    preferred_username: String,
    email: String,
    given_name: Option<String>,
    family_name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct KeycloakRole {
    id: Option<String>,
    name: String,
    description: Option<String>,
    composite: Option<bool>,
    #[serde(rename = "clientRole")]
    client_role: Option<bool>,
    #[serde(rename = "containerId")]
    container_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KeycloakUser {
    id: Option<String>,
    username: Option<String>,
    email: Option<String>,
    #[serde(rename = "firstName")]
    first_name: Option<String>,
    #[serde(rename = "lastName")]
    last_name: Option<String>,
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
