use super::super::transport::{self, KeycloakRole, KeycloakUser};
use super::KeycloakIdentityProvider;
use super::mapping;
use crate::auth::{IdentityError, ManagedUser};

impl KeycloakIdentityProvider {
    pub(super) async fn pending_users_impl(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        Ok(self
            .managed_users_impl()
            .await?
            .into_iter()
            .filter(|user| {
                !user
                    .realm_roles
                    .iter()
                    .any(|role| role == &self.config.user_role)
            })
            .collect())
    }

    pub(super) async fn managed_users_impl(&self) -> Result<Vec<ManagedUser>, IdentityError> {
        let token = self.client_token().await?;
        let users: Vec<KeycloakUser> = self
            .get_bearer(&format!("admin/realms/{}/users", self.config.realm), &token)
            .await?;
        let mut managed = Vec::new();
        for user in users {
            let Some(id) = user.id.as_deref() else {
                continue;
            };
            let roles: Vec<KeycloakRole> = self
                .get_bearer(
                    &format!(
                        "admin/realms/{}/users/{id}/role-mappings/realm",
                        self.config.realm
                    ),
                    &token,
                )
                .await?;
            if roles.iter().any(|role| role.name == "admin") {
                continue;
            }
            managed.push(mapping::managed_user(user, None, roles));
        }
        Ok(managed)
    }

    pub(super) async fn user_by_id_impl(
        &self,
        user_id: &str,
    ) -> Result<ManagedUser, IdentityError> {
        let token = self.client_token().await?;
        let user: KeycloakUser = self
            .get_bearer(
                &format!("admin/realms/{}/users/{user_id}", self.config.realm),
                &token,
            )
            .await?;
        Ok(mapping::managed_user(user, Some(user_id), Vec::new()))
    }

    pub(super) async fn delete_user_impl(&self, user_id: &str) -> Result<(), IdentityError> {
        let token = self.client_token().await?;
        let response = self
            .client
            .delete(self.endpoint(&format!(
                "admin/realms/{}/users/{user_id}",
                self.config.realm
            ))?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(transport::transport_error)?;
        transport::decode_empty_response(response).await
    }
}
