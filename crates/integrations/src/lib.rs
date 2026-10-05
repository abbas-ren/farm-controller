//! External service ports and their HTTP adapters.
//!
//! The service crate depends on these adapters, while this crate depends only
//! on shared configuration and HTTP policy from `farmcontroller-core`.

pub use farmcontroller_core::{config, external_http};

pub mod jira;
pub mod qmetry_catalog;
pub mod test_catalog;
