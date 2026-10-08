pub mod admin_control;
pub mod api;
pub mod auth;
pub mod devices;
pub mod events;
pub mod observability;
pub mod persistence;
pub mod reports;
pub mod runtime_log_handlers;
pub mod state;
pub mod test_results;
pub mod workers;

pub use farmcontroller_core::{cli, config, error, external_http};
pub use farmcontroller_integrations::{jira, qmetry_catalog, test_catalog};

use std::sync::Arc;

use axum::Router;
use state::AppState;

pub fn build_app(state: Arc<AppState>) -> Router {
    api::router(state)
}
