//! Keycloak client setup and reusable token and bearer-request operations.

use std::time::Duration;

use axum::http::StatusCode;
use reqwest::Url;
use serde::de::DeserializeOwned;

use super::{KeycloakIdentityProvider, transport};
use crate::auth::{IdentityError, RefreshedTokens};
use crate::{config::AuthConfig, error::AppError};

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

    pub(super) fn endpoint(&self, path: &str) -> Result<Url, IdentityError> {
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

    pub(super) async fn token_request(
        &self,
        form: &[(&str, &str)],
    ) -> Result<transport::TokenResponse, IdentityError> {
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
            .map_err(transport::transport_error)?;
        transport::decode_response(response).await
    }

    pub(super) async fn client_token(&self) -> Result<String, IdentityError> {
        let response = self
            .token_request(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.config.client_id),
                ("client_secret", self.client_secret()?),
            ])
            .await?;
        Ok(response.access_token)
    }

    pub(super) async fn password_token(
        &self,
        username: &str,
        password: &str,
    ) -> Result<transport::TokenResponse, IdentityError> {
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

    pub(super) async fn introspect(
        &self,
        token: &str,
    ) -> Result<transport::IntrospectionResponse, IdentityError> {
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
            .map_err(transport::transport_error)?;
        transport::decode_response(response).await
    }

    pub(super) async fn refresh(
        &self,
        refresh_token: &str,
    ) -> Result<RefreshedTokens, IdentityError> {
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

    pub(super) async fn get_bearer<T: DeserializeOwned>(
        &self,
        path: &str,
        token: &str,
    ) -> Result<T, IdentityError> {
        self.get_bearer_url(self.endpoint(path)?, token).await
    }

    pub(super) async fn get_bearer_url<T: DeserializeOwned>(
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
            .map_err(transport::transport_error)?;
        transport::decode_response(response).await
    }
}
