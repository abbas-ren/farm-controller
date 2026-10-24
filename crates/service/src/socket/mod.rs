//! Farm WebSocket surface: upgrade, rooms, and typed event routes.
//!
//! Bridges edge agents/controllers, terminals, web CLI, and frontend listeners
//! through the shared `EventHub` and axum upgrade handler.

#[cfg(test)]
pub mod tests;

pub mod constants;
pub mod helpers;
pub mod impls;
pub mod routes;
pub mod types;
