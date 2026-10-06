use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, method, path, path_regex},
};

use super::support::{mount_admin_token, test_provider};
use crate::auth::{IdentityProvider, RegistrationAction, RegistrationActionRequest};

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
        .register_user(&crate::auth::AdminRegistrationRequest {
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
