pub mod api;
pub mod auth;
pub mod cli;
pub mod config;
pub mod devices;
pub mod error;
pub mod events;
pub mod external_http;
pub mod jira;
pub mod observability;
pub mod persistence;
pub mod qmetry_catalog;
pub mod reports;
pub mod state;
pub mod test_catalog;
pub mod test_results;
pub mod workers;

use std::sync::Arc;

use axum::Router;
use state::AppState;

pub fn build_app(state: Arc<AppState>) -> Router {
    api::router(state)
}
