use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use clap::Parser;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, method, path},
};

use super::super::*;
use crate::{
    cli::Cli,
    config::{AppConfig, AuthConfig},
    observability::Metrics,
    state::AppState,
};

pub(super) struct FixtureIdentityProvider {
    pub(super) signin_error: Option<IdentityError>,
}

#[async_trait]
impl IdentityProvider for FixtureIdentityProvider {
    async fn signin(
        &self,
        _username: &str,
        _password: &str,
    ) -> Result<SigninResult, IdentityError> {
        if let Some(error) = &self.signin_error {
            return Err(IdentityError {
                status: error.status,
                message: error.message.clone(),
            });
        }
        Ok(SigninResult {
            access_token: "access-token".to_owned(),
            refresh_token: "refresh-token".to_owned(),
            user: AuthenticatedUser {
                id: "user-id".to_owned(),
                username: "farm.user".to_owned(),
                email: "farm.user@example.com".to_owned(),
                first_name: Some("Farm".to_owned()),
                last_name: Some("User".to_owned()),
                realm_roles: vec!["user".to_owned()],
            },
        })
    }

    async fn validate(
        &self,
        _access_token: &str,
        _refresh_token: Option<&str>,
        _required_role: Option<&str>,
    ) -> Result<ValidationResult, IdentityError> {
        Ok(ValidationResult {
            refreshed_tokens: None,
        })
    }

    async fn request_registration(
        &self,
        request: &RegistrationRequest,
    ) -> Result<serde_json::Value, IdentityError> {
        Ok(serde_json::json!({"username": request.username, "enabled": true}))
    }

    async fn pending_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        Ok(vec![fixture_user("pending", Vec::new())])
    }

    async fn managed_users(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        Ok(vec![
            fixture_user("pending", Vec::new()),
            fixture_user("approved", vec!["user".to_owned()]),
        ])
    }

    async fn user_by_id(&self, user_id: &str) -> Result<ManagedUser, IdentityError> {
        Ok(fixture_user(user_id, vec!["user".to_owned()]))
    }

    async fn delete_user(&self, _user_id: &str) -> Result<(), IdentityError> {
        Ok(())
    }

    async fn verify_reset_user(&self, _token: &str, _user_id: &str) -> Result<(), IdentityError> {
        Ok(())
    }

    async fn reset_password(
        &self,
        _token: &str,
        _user_id: &str,
        _password: &str,
    ) -> Result<(), IdentityError> {
        Ok(())
    }

    async fn forgot_password(&self, _username: &str) -> Result<(), IdentityError> {
        Ok(())
    }

    async fn register_user(
        &self,
        request: &AdminRegistrationRequest,
    ) -> Result<AdminRegistrationResult, IdentityError> {
        Ok(AdminRegistrationResult {
            username: request.username.clone(),
            assigned_role: request.role.clone(),
            user_data: serde_json::json!({"id": "created-user"}),
        })
    }

    async fn registration_action(
        &self,
        _request: &RegistrationActionRequest,
    ) -> Result<(), IdentityError> {
        Ok(())
    }
}

pub(super) fn fixture_user(id: &str, realm_roles: Vec<String>) -> ManagedUser {
    ManagedUser {
        id: id.to_owned(),
        username: format!("{id}.user"),
        email: Some(format!("{id}@example.com")),
        first_name: Some("Farm".to_owned()),
        last_name: Some("User".to_owned()),
        realm_roles,
    }
}

pub(super) fn handler_app(provider: Option<FixtureIdentityProvider>) -> Router {
    let mut config = AppConfig::load(&Cli::try_parse_from(["farmcontroller"]).unwrap()).unwrap();
    config.modules.enabled.insert(crate::config::Module::Auth);
    config.auth.secure_cookies = false;
    let state = match provider {
        Some(provider) => {
            AppState::with_identity_provider(config, Metrics::new().unwrap(), Arc::new(provider))
        }
        None => AppState::without_dependencies(config, Metrics::new().unwrap()),
    };
    router().with_state(Arc::new(state))
}

pub(super) async fn mount_admin_token(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/realms/dev-realm/protocol/openid-connect/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "admin-token", "refresh_token": "unused"
        })))
        .mount(server)
        .await;
}

pub(super) fn test_provider(server: &MockServer) -> KeycloakIdentityProvider {
    KeycloakIdentityProvider::new(AuthConfig {
        keycloak_url: server.uri(),
        realm: "dev-realm".to_owned(),
        client_id: "dev-auth".to_owned(),
        client_secret: Some("secret".to_owned()),
        user_role: "user".to_owned(),
        default_user_password: "Renesas123".to_owned(),
        request_timeout_seconds: 2,
        secure_cookies: false,
    })
    .unwrap()
}
