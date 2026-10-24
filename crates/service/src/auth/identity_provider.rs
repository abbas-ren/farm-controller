//! Provider abstraction used by handlers and alternate authentication backends.

use async_trait::async_trait;
use axum::http::StatusCode;

use super::{
    AdminRegistrationRequest, AdminRegistrationResult, IdentityError, ManagedUser,
    RegistrationActionRequest, RegistrationRequest, SigninResult, ValidationResult,
};

/// Identity operations required by the authentication HTTP API.
#[async_trait]
pub trait IdentityProvider: Send + Sync {
    /// Signin.
    async fn signin(&self, username: &str, password: &str) -> Result<SigninResult, IdentityError>;
    /// Validate.
    async fn validate(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        required_role: Option<&str>,
    ) -> Result<ValidationResult, IdentityError>;
    /// Request registration.
    async fn request_registration(
        &self,
        _request: &RegistrationRequest,
    ) -> Result<serde_json::Value, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::identity_provider", "request_registration");
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "Registration is unavailable",
        ))
    }
    /// Pending users.
    async fn pending_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::identity_provider", "pending_users");
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
    /// Managed users.
    async fn managed_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::identity_provider", "managed_users");
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
    /// User by id.
    async fn user_by_id(&self, _user_id: &str) -> Result<ManagedUser, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::identity_provider", "user_by_id");
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
    /// Delete user.
    async fn delete_user(&self, _user_id: &str) -> Result<(), IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::identity_provider", "delete_user");
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
    /// Verify reset user.
    async fn verify_reset_user(&self, _token: &str, _user_id: &str) -> Result<(), IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::identity_provider", "verify_reset_user");
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "Password reset is unavailable",
        ))
    }
    /// Reset password.
    async fn reset_password(
        &self,
        _token: &str,
        _user_id: &str,
        _password: &str,
    ) -> Result<(), IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::identity_provider", "reset_password");
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "Password reset is unavailable",
        ))
    }
    /// Forgot password.
    async fn forgot_password(&self, _username: &str) -> Result<(), IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::identity_provider", "forgot_password");
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "Password reset is unavailable",
        ))
    }
    /// Register user.
    async fn register_user(
        &self,
        _request: &AdminRegistrationRequest,
    ) -> Result<AdminRegistrationResult, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::identity_provider", "register_user");
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
    /// Registration action.
    async fn registration_action(
        &self,
        _request: &RegistrationActionRequest,
    ) -> Result<(), IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::identity_provider", "registration_action");
        Err(IdentityError::upstream(
            StatusCode::NOT_IMPLEMENTED,
            "User administration is unavailable",
        ))
    }
}
