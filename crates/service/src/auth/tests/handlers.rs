use axum::{
    body::Body,
    http::{Request, StatusCode, header::SET_COOKIE},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::support::{FixtureIdentityProvider, handler_app};
use crate::auth::constants::{INVALID_CREDENTIALS, SIGNIN_SUCCESS};

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
                signin_error: Some(crate::auth::IdentityError {
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
