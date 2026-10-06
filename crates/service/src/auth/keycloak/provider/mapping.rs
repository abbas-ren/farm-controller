use super::super::transport::{KeycloakRole, KeycloakUser, TokenResponse, UserInfo};
use crate::auth::{AuthenticatedUser, ManagedUser, RegistrationRequest, SigninResult};

pub(super) fn signin_result(
    token: TokenResponse,
    user_info: UserInfo,
    roles: Vec<KeycloakRole>,
) -> SigninResult {
    SigninResult {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        user: AuthenticatedUser {
            id: user_info.sub,
            username: user_info.preferred_username,
            email: user_info.email,
            first_name: user_info.given_name,
            last_name: user_info.family_name,
            realm_roles: roles.into_iter().map(|role| role.name).collect(),
        },
    }
}

pub(super) fn managed_user(
    user: KeycloakUser,
    fallback_id: Option<&str>,
    roles: Vec<KeycloakRole>,
) -> ManagedUser {
    ManagedUser {
        id: user
            .id
            .or_else(|| fallback_id.map(str::to_owned))
            .unwrap_or_default(),
        username: user.username.unwrap_or_default(),
        email: user.email,
        first_name: user.first_name,
        last_name: user.last_name,
        realm_roles: roles.into_iter().map(|role| role.name).collect(),
    }
}

pub(super) fn pending_registration_response(
    user_id: &str,
    request: &RegistrationRequest,
) -> serde_json::Value {
    serde_json::json!({
        "id": user_id,
        "username": request.username,
        "firstName": request.first_name,
        "lastName": request.last_name,
        "email": request.email,
        "emailVerified": false,
        "createdTimestamp": chrono::Utc::now().timestamp_millis(),
        "enabled": true,
        "totp": false,
        "disableableCredentialTypes": [],
        "requiredActions": ["UPDATE_PASSWORD"],
        "notBefore": 0,
        "access": {
            "manageGroupMembership": true,
            "view": true,
            "mapRoles": true,
            "impersonate": true,
            "manage": true
        },
        "realmRoles": []
    })
}
