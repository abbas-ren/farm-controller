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
