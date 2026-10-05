use std::sync::Arc;

use axum::{Router, body::Body, http::Request};
use clap::Parser;
use http_body_util::BodyExt;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, header, method, path, path_regex},
};

use super::*;
use crate::{cli::Cli, config::AppConfig, observability::Metrics, state::AppState};

struct FixtureIdentityProvider {
    signin_error: Option<IdentityError>,
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

fn fixture_user(id: &str, realm_roles: Vec<String>) -> ManagedUser {
    ManagedUser {
        id: id.to_owned(),
        username: format!("{id}.user"),
        email: Some(format!("{id}@example.com")),
        first_name: Some("Farm".to_owned()),
        last_name: Some("User".to_owned()),
        realm_roles,
    }
}

fn handler_app(provider: Option<FixtureIdentityProvider>) -> Router {
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

#[tokio::test]
async fn signin_handler_preserves_frontend_envelope_and_cookies() {
    let response = handler_app(Some(FixtureIdentityProvider { signin_error: None }))
        .oneshot(
            Request::post("/signin")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"emailOrUsername":"farm.user","password":"secret"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let cookies = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        cookies
            .iter()
            .any(|value| value.starts_with("accessToken=access-token"))
    );
    assert!(
        cookies
            .iter()
            .any(|value| value.starts_with("refreshToken=refresh-token"))
    );
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["message"], SIGNIN_SUCCESS);
    assert_eq!(body["token"], "access-token");
    assert_eq!(body["refreshToken"], "refresh-token");
    assert_eq!(body["user"]["realmRoles"][0], "user");
}

#[tokio::test]
async fn signin_handler_maps_validation_upstream_and_disabled_errors() {
    for (app, payload, expected) in [
        (
            handler_app(Some(FixtureIdentityProvider { signin_error: None })),
            r#"{"emailOrUsername":"","password":""}"#,
            StatusCode::BAD_REQUEST,
        ),
        (
            handler_app(Some(FixtureIdentityProvider {
                signin_error: Some(IdentityError {
                    status: StatusCode::UNAUTHORIZED,
                    message: INVALID_CREDENTIALS.to_owned(),
                }),
            })),
            r#"{"emailOrUsername":"farm.user","password":"wrong"}"#,
            StatusCode::UNAUTHORIZED,
        ),
        (
            handler_app(None),
            r#"{"emailOrUsername":"farm.user","password":"secret"}"#,
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    ] {
        let response = app
            .oneshot(
                Request::post("/signin")
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["success"], false);
    }
}

#[tokio::test]
async fn mounted_auth_routes_preserve_success_contracts() {
    let app = handler_app(Some(FixtureIdentityProvider { signin_error: None }));
    for (method, path, body, expected) in [
        (
            "POST",
            "/register/request",
            Some(
                r#"{"username":"farm.user","email":"farm@example.com","firstName":"Farm","lastName":"User"}"#,
            ),
            StatusCode::CREATED,
        ),
        (
            "POST",
            "/forgot-password",
            Some(r#"{"emailOrUsername":"farm.user"}"#),
            StatusCode::OK,
        ),
        (
            "POST",
            "/verify-user",
            Some(r#"{"token":"reset-token","userId":"user-id"}"#),
            StatusCode::OK,
        ),
        (
            "POST",
            "/reset-password",
            Some(r#"{"token":"reset-token","userId":"user-id","password":"new-password"}"#),
            StatusCode::OK,
        ),
        (
            "POST",
            "/register",
            Some(
                r#"{"username":"farm.admin","email":"admin@example.com","firstName":"Farm","lastName":"Admin","role":"admin"}"#,
            ),
            StatusCode::CREATED,
        ),
        (
            "POST",
            "/register/action",
            Some(r#"{"userId":"user-id","action":"Accept"}"#),
            StatusCode::OK,
        ),
        (
            "POST",
            "/user-registration/confirmation/send-mail",
            Some(r#"{"userId":"user-id","eventType":"REGISTER"}"#),
            StatusCode::OK,
        ),
        ("GET", "/users/requests", None, StatusCode::OK),
        ("GET", "/users", None, StatusCode::OK),
        ("GET", "/users/active", None, StatusCode::OK),
        ("GET", "/user/user-id", None, StatusCode::OK),
        ("DELETE", "/user/user-id", None, StatusCode::OK),
        ("GET", "/validate", None, StatusCode::OK),
        ("GET", "/validate/admin", None, StatusCode::OK),
        ("GET", "/validate/user", None, StatusCode::OK),
    ] {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", "Bearer access-token");
        let request_body = match body {
            Some(body) => {
                request = request.header("content-type", "application/json");
                Body::from(body)
            }
            None => Body::empty(),
        };
        let response = app
            .clone()
            .oneshot(request.body(request_body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{method} {path}");
    }
}

#[tokio::test]
async fn mounted_auth_routes_reject_malformed_or_unauthorized_requests() {
    let app = handler_app(Some(FixtureIdentityProvider { signin_error: None }));
    for (path, body) in [
        (
            "/register/request",
            r#"{"username":"","email":"farm@example.com","firstName":"Farm","lastName":"User"}"#,
        ),
        ("/forgot-password", r#"{"emailOrUsername":""}"#),
        ("/verify-user", r#"{"token":"","userId":""}"#),
        (
            "/reset-password",
            r#"{"token":"","userId":"","password":""}"#,
        ),
        (
            "/register",
            r#"{"username":"farm.user","email":"farm@example.com","firstName":"Farm","lastName":"User","role":"operator"}"#,
        ),
        ("/register/action", r#"{"userId":"","action":"Accept"}"#),
        (
            "/user-registration/confirmation/send-mail",
            r#"{"userId":"","eventType":""}"#,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "POST {path}");
    }

    for (method, path, body) in [
        ("GET", "/users/requests", None),
        ("GET", "/users", None),
        ("GET", "/users/active", None),
        ("DELETE", "/user/user-id", None),
        ("GET", "/validate", None),
        ("GET", "/validate/admin", None),
        ("GET", "/validate/user", None),
        (
            "POST",
            "/register",
            Some(
                r#"{"username":"farm.admin","email":"admin@example.com","firstName":"Farm","lastName":"Admin","role":"admin"}"#,
            ),
        ),
        (
            "POST",
            "/register/action",
            Some(r#"{"userId":"user-id","action":"Accept"}"#),
        ),
    ] {
        let mut request = Request::builder().method(method).uri(path);
        let request_body = match body {
            Some(body) => {
                request = request.header("content-type", "application/json");
                Body::from(body)
            }
            None => Body::empty(),
        };
        let response = app
            .clone()
            .oneshot(request.body(request_body).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
    }
}

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
        .request_registration(&RegistrationRequest {
            username: "farm.user".to_owned(),
            email: "farm.user@example.com".to_owned(),
            first_name: "Farm".to_owned(),
            last_name: "User".to_owned(),
        })
        .await
        .unwrap();
}

async fn mount_admin_token(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/realms/dev-realm/protocol/openid-connect/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "admin-token", "refresh_token": "unused"
        })))
        .mount(server)
        .await;
}

fn test_provider(server: &MockServer) -> KeycloakIdentityProvider {
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

#[tokio::test]
async fn keycloak_pending_users_excludes_approved_and_admin_users() {
    let server = MockServer::start().await;
    mount_admin_token(&server).await;
    Mock::given(method("GET"))
        .and(path("/admin/realms/dev-realm/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": "pending", "username": "pending.user", "email": "pending@example.com"},
            {"id": "approved", "username": "approved.user"},
            {"id": "administrator", "username": "admin.user"}
        ])))
        .mount(&server)
        .await;
    for (id, role) in [
        ("pending", None),
        ("approved", Some("user")),
        ("administrator", Some("admin")),
    ] {
        let body = role.map_or_else(
            || serde_json::json!([]),
            |role| serde_json::json!([{"name": role}]),
        );
        Mock::given(method("GET"))
            .and(path(format!(
                "/admin/realms/dev-realm/users/{id}/role-mappings/realm"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }
    let users = test_provider(&server).pending_users().await.unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].id, "pending");
}

#[tokio::test]
async fn keycloak_acceptance_assigns_user_role_and_default_password() {
    let server = MockServer::start().await;
    mount_admin_token(&server).await;
    Mock::given(method("GET"))
        .and(path("/admin/realms/dev-realm/roles/user"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "role-id", "name": "user"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(
            "/admin/realms/dev-realm/users/pending/role-mappings/realm",
        ))
        .and(body_string_contains("\"name\":\"user\""))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/admin/realms/dev-realm/users/pending/reset-password"))
        .and(body_string_contains("\"value\":\"Renesas123\""))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    test_provider(&server)
        .registration_action(&RegistrationActionRequest {
            user_id: "pending".to_owned(),
            action: RegistrationAction::Accept,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn keycloak_forgot_password_uses_single_purpose_action_email() {
    let server = MockServer::start().await;
    mount_admin_token(&server).await;
    Mock::given(method("GET"))
        .and(path("/admin/realms/dev-realm/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": "user-id", "username": "farm.user", "email": "farm@example.com"}
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(
            "/admin/realms/dev-realm/users/user-id/execute-actions-email",
        ))
        .and(body_string_contains("UPDATE_PASSWORD"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    test_provider(&server)
        .forgot_password("farm.user")
        .await
        .unwrap();
}

#[tokio::test]
async fn keycloak_admin_registration_creates_role_and_password() {
    let server = MockServer::start().await;
    mount_admin_token(&server).await;
    Mock::given(method("POST"))
        .and(path("/admin/realms/dev-realm/users"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/admin/realms/dev-realm/roles/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "role-id", "name": "user"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"name\":\"user\""))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(body_string_contains("\"value\":\"Renesas123\""))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(body_string_contains("\"emailVerified\":true"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/admin/realms/dev-realm/users/[0-9a-f-]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "created", "username": "created.user", "emailVerified": true
        })))
        .mount(&server)
        .await;

    let result = test_provider(&server)
        .register_user(&AdminRegistrationRequest {
            username: "created.user".to_owned(),
            email: "created@example.com".to_owned(),
            first_name: "Created".to_owned(),
            last_name: "User".to_owned(),
            role: "user".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(result.assigned_role, "user");
    assert_eq!(result.user_data["realmRoles"], serde_json::json!(["user"]));
}
