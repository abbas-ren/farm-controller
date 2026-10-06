//! Request, response, and identity result types used by the authentication API.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
