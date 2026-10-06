use axum::http::StatusCode;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, header, method, path},
};

use crate::{
    auth::{IdentityProvider, KeycloakIdentityProvider},
    config::AuthConfig,
};

#[tokio::test]
async fn keycloak_signin_preserves_frontend_user_shape() {
    let server = MockServer::start().await;
    let token_path = "/realms/dev-realm/protocol/openid-connect/token";
    Mock::given(method("POST"))
        .and(path(token_path))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "admin-token",
            "refresh_token": "unused",
            "expires_in": 900
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(token_path))
        .and(body_string_contains("grant_type=password"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "expires_in": 900
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/realms/dev-realm/protocol/openid-connect/userinfo"))
        .and(header("authorization", "Bearer access-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "sub": "user-id",
            "preferred_username": "farm.user",
            "email": "farm.user@example.com",
            "given_name": "Farm",
            "family_name": "User"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/admin/realms/dev-realm/users/user-id/role-mappings/realm",
        ))
        .and(header("authorization", "Bearer admin-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "name": "user" }
        ])))
        .mount(&server)
        .await;

    let provider = KeycloakIdentityProvider::new(AuthConfig {
        keycloak_url: server.uri(),
        realm: "dev-realm".to_owned(),
        client_id: "dev-auth".to_owned(),
        client_secret: Some("secret".to_owned()),
        user_role: "user".to_owned(),
        default_user_password: "Renesas123".to_owned(),
        request_timeout_seconds: 2,
        secure_cookies: false,
    })
    .unwrap();
    let result = provider.signin("farm.user", "password").await.unwrap();

    assert_eq!(result.access_token, "access-token");
    assert_eq!(result.user.id, "user-id");
    assert_eq!(result.user.realm_roles, ["user"]);
}

#[tokio::test]
async fn keycloak_validation_enforces_realm_roles() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/realms/dev-realm/protocol/openid-connect/token/introspect",
        ))
        .and(body_string_contains("token=active-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "active": true,
            "realm_access": { "roles": ["user"] }
        })))
        .mount(&server)
        .await;
    let provider = KeycloakIdentityProvider::new(AuthConfig {
        keycloak_url: server.uri(),
        realm: "dev-realm".to_owned(),
        client_id: "dev-auth".to_owned(),
        client_secret: Some("secret".to_owned()),
        user_role: "user".to_owned(),
        default_user_password: "Renesas123".to_owned(),
        request_timeout_seconds: 2,
        secure_cookies: false,
    })
    .unwrap();

    provider
        .validate("active-token", None, Some("user"))
        .await
        .unwrap();
    let error = provider
        .validate("active-token", None, Some("admin"))
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn keycloak_registration_creates_pending_enabled_user() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/realms/dev-realm/protocol/openid-connect/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "admin-token", "refresh_token": "unused"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/admin/realms/dev-realm/users"))
        .and(header("authorization", "Bearer admin-token"))
        .and(body_string_contains(
            "\"requiredActions\":[\"UPDATE_PASSWORD\"]",
        ))
        .and(body_string_contains("\"emailVerified\":false"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;
    let provider = KeycloakIdentityProvider::new(AuthConfig {
        keycloak_url: server.uri(),
        realm: "dev-realm".to_owned(),
        client_id: "dev-auth".to_owned(),
        client_secret: Some("secret".to_owned()),
        user_role: "user".to_owned(),
        default_user_password: "Renesas123".to_owned(),
        request_timeout_seconds: 2,
        secure_cookies: false,
    })
    .unwrap();
    provider
        .request_registration(&crate::auth::RegistrationRequest {
            username: "farm.user".to_owned(),
            email: "farm.user@example.com".to_owned(),
            first_name: "Farm".to_owned(),
            last_name: "User".to_owned(),
        })
        .await
        .unwrap();
}
