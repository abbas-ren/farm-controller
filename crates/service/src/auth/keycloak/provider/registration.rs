use super::super::transport::{self, KeycloakRole};
use super::KeycloakIdentityProvider;
use super::mapping;
use crate::auth::{
    AdminRegistrationRequest, AdminRegistrationResult, IdentityError, RegistrationAction,
    RegistrationActionRequest, RegistrationRequest,
};

impl KeycloakIdentityProvider {
    pub(super) async fn request_registration_impl(
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
            .map_err(transport::transport_error)?;
        transport::decode_empty_response(response).await?;
        Ok(mapping::pending_registration_response(&user_id, request))
    }

    pub(super) async fn register_user_impl(
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
            .map_err(transport::transport_error)?;
        transport::decode_empty_response(response).await?;

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
            .map_err(transport::transport_error)?;
        transport::decode_empty_response(response).await?;

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
                .map_err(transport::transport_error)?;
            transport::decode_empty_response(response).await?;
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

    pub(super) async fn registration_action_impl(
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
                    .map_err(transport::transport_error)?;
                transport::decode_empty_response(response).await
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
                    .map_err(transport::transport_error)?;
                transport::decode_empty_response(response).await?;
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
                    .map_err(transport::transport_error)?;
                transport::decode_empty_response(response).await
            }
        }
    }
}
