//! Authentication API, identity-provider abstraction, and Keycloak adapter.
//!
//! Public types and handlers are re-exported here to preserve the service's
//! established `auth::...` import paths.

mod constants;
mod errors;
mod handlers;
mod identity_provider;
mod keycloak;
mod models;
mod session;
mod validation;

#[cfg(test)]
pub mod tests;

pub use errors::{AuthorizationError, IdentityError};
pub use handlers::{
    active_users, all_users, delete_user, forgot_password, pending_users, register_user,
    registration_action, request_registration, reset_password, router, signin, user_by_id,
    user_event_webhook, validate, validate_admin, validate_user, verify_user,
};
pub use identity_provider::IdentityProvider;
pub use keycloak::KeycloakIdentityProvider;
pub use models::*;
pub use session::authorize_request;
pub(crate) use session::token_subject;

pub(crate) use handlers::{
    __path_active_users, __path_all_users, __path_delete_user, __path_forgot_password,
    __path_pending_users, __path_register_user, __path_registration_action,
    __path_request_registration, __path_reset_password, __path_signin, __path_user_by_id,
    __path_user_event_webhook, __path_validate, __path_validate_admin, __path_validate_user,
    __path_verify_user,
};
