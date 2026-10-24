//! Keycloak identity-provider operations, grouped by capability.

mod managed_users;
mod mapping;
mod password;
mod registration;
mod session;

use async_trait::async_trait;

use crate::auth::{
    AdminRegistrationRequest, AdminRegistrationResult, IdentityError, IdentityProvider,
    ManagedUser, RegistrationActionRequest, RegistrationRequest, SigninResult, ValidationResult,
};

pub struct KeycloakIdentityProvider {
    pub(super) client: reqwest::Client,
    pub(super) base_url: reqwest::Url,
    pub(super) config: crate::config::AuthConfig,
}

#[async_trait]
impl IdentityProvider for KeycloakIdentityProvider {
    /// Signin.
    async fn signin(&self, username: &str, password: &str) -> Result<SigninResult, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "signin");
        self.signin_impl(username, password).await
    }

    /// Validate.
    async fn validate(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        required_role: Option<&str>,
    ) -> Result<ValidationResult, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "validate");
        self.validate_impl(access_token, refresh_token, required_role)
            .await
    }

    /// Request registration.
    async fn request_registration(
        &self,
        request: &RegistrationRequest,
    ) -> Result<serde_json::Value, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "request_registration");
        self.request_registration_impl(request).await
    }

    /// Pending users.
    async fn pending_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "pending_users");
        self.pending_users_impl().await
    }

    /// Managed users.
    async fn managed_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "managed_users");
        self.managed_users_impl().await
    }

    /// User by id.
    async fn user_by_id(&self, user_id: &str) -> Result<ManagedUser, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "user_by_id");
        self.user_by_id_impl(user_id).await
    }

    /// Delete user.
    async fn delete_user(&self, user_id: &str) -> Result<(), IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "delete_user");
        self.delete_user_impl(user_id).await
    }

    /// Verify reset user.
    async fn verify_reset_user(&self, token: &str, user_id: &str) -> Result<(), IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "verify_reset_user");
        self.verify_reset_user_impl(token, user_id).await
    }

    /// Reset password.
    async fn reset_password(
        &self,
        token: &str,
        user_id: &str,
        password: &str,
    ) -> Result<(), IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "reset_password");
        self.reset_password_impl(token, user_id, password).await
    }

    /// Forgot password.
    async fn forgot_password(&self, username: &str) -> Result<(), IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "forgot_password");
        self.forgot_password_impl(username).await
    }

    /// Register user.
    async fn register_user(
        &self,
        request: &AdminRegistrationRequest,
    ) -> Result<AdminRegistrationResult, IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "register_user");
        self.register_user_impl(request).await
    }

    /// Registration action.
    async fn registration_action(
        &self,
        request: &RegistrationActionRequest,
    ) -> Result<(), IdentityError> {
        let _api_trace = crate::observability::ApiTrace::enter("auth::keycloak::provider::mod", "registration_action");
        self.registration_action_impl(request).await
    }
}
