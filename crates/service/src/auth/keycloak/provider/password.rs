use axum::http::StatusCode;

use super::super::transport::{self, KeycloakUser};
use super::KeycloakIdentityProvider;
use crate::auth::IdentityError;

impl KeycloakIdentityProvider {
    pub(super) async fn verify_reset_user_impl(
        &self,
        token: &str,
        user_id: &str,
    ) -> Result<(), IdentityError> {
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

    pub(super) async fn reset_password_impl(
        &self,
        token: &str,
        user_id: &str,
        password: &str,
    ) -> Result<(), IdentityError> {
        self.verify_reset_user_impl(token, user_id).await?;
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
            .map_err(transport::transport_error)?;
        transport::decode_empty_response(response).await?;
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
            .map_err(transport::transport_error)?;
        transport::decode_empty_response(response).await
    }

    pub(super) async fn forgot_password_impl(&self, username: &str) -> Result<(), IdentityError> {
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
            .map_err(transport::transport_error)?;
        transport::decode_empty_response(response).await
    }
}
