use axum::http::StatusCode;

use super::super::transport::{KeycloakRole, KeycloakUser, UserInfo};
use super::KeycloakIdentityProvider;
use super::mapping;
use crate::auth::constants::{FORBIDDEN, INVALID_CREDENTIALS, SESSION_EXPIRED};
use crate::auth::{IdentityError, SigninResult, ValidationResult};

impl KeycloakIdentityProvider {
    pub(super) async fn signin_impl(
        &self,
        username: &str,
        password: &str,
    ) -> Result<SigninResult, IdentityError> {
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

        Ok(mapping::signin_result(token, user_info, roles))
    }

    pub(super) async fn validate_impl(
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
}
