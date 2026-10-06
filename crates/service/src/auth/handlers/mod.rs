//! HTTP routes and endpoint handlers for authentication workflows.

mod registration;
mod sessions;
mod users;

pub use registration::{
    __path_forgot_password, __path_register_user, __path_registration_action,
    __path_request_registration, __path_reset_password, __path_user_event_webhook,
    __path_verify_user, forgot_password, register_user, registration_action, request_registration,
    reset_password, user_event_webhook, verify_user,
};
pub use sessions::{
    __path_signin, __path_validate, __path_validate_admin, __path_validate_user, signin, validate,
    validate_admin, validate_user,
};
pub use users::{
    __path_active_users, __path_all_users, __path_delete_user, __path_pending_users,
    __path_user_by_id, active_users, all_users, delete_user, pending_users, user_by_id,
};

use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};

use crate::state::AppState;

/// Builds the authentication subrouter mounted by the service API.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/register/request", post(request_registration))
        .route("/register/action", post(registration_action))
        .route("/users/requests", get(pending_users))
        .route("/users", get(all_users))
        .route("/users/active", get(active_users))
        .route("/user/{user_id}", get(user_by_id).delete(delete_user))
        .route("/verify-user", post(verify_user))
        .route("/reset-password", post(reset_password))
        .route("/forgot-password", post(forgot_password))
        .route("/register", post(register_user))
        .route(
            "/user-registration/confirmation/send-mail",
            post(user_event_webhook),
        )
        .route("/signin", post(signin))
        .route("/validate", get(validate))
        .route("/validate/admin", get(validate_admin))
        .route("/validate/user", get(validate_user))
}
