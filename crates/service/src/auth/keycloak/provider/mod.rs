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
    async fn signin(&self, username: &str, password: &str) -> Result<SigninResult, IdentityError> {
        self.signin_impl(username, password).await
    }

    async fn validate(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        required_role: Option<&str>,
    ) -> Result<ValidationResult, IdentityError> {
        self.validate_impl(access_token, refresh_token, required_role)
            .await
    }

    async fn request_registration(
        &self,
        request: &RegistrationRequest,
    ) -> Result<serde_json::Value, IdentityError> {
        self.request_registration_impl(request).await
    }

    async fn pending_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        self.pending_users_impl().await
    }

    async fn managed_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        self.managed_users_impl().await
    }

    async fn user_by_id(&self, user_id: &str) -> Result<ManagedUser, IdentityError> {
        self.user_by_id_impl(user_id).await
    }

    async fn delete_user(&self, user_id: &str) -> Result<(), IdentityError> {
        self.delete_user_impl(user_id).await
    }

    async fn verify_reset_user(&self, token: &str, user_id: &str) -> Result<(), IdentityError> {
        self.verify_reset_user_impl(token, user_id).await
    }

    async fn reset_password(
        &self,
        token: &str,
        user_id: &str,
        password: &str,
    ) -> Result<(), IdentityError> {
        self.reset_password_impl(token, user_id, password).await
    }

    async fn forgot_password(&self, username: &str) -> Result<(), IdentityError> {
        self.forgot_password_impl(username).await
    }

    async fn register_user(
        &self,
        request: &AdminRegistrationRequest,
    ) -> Result<AdminRegistrationResult, IdentityError> {
        self.register_user_impl(request).await
    }

    async fn registration_action(
        &self,
        request: &RegistrationActionRequest,
    ) -> Result<(), IdentityError> {
        self.registration_action_impl(request).await
    }
}
