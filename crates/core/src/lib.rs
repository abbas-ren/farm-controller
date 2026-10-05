//! Shared configuration and infrastructure policies.
//!
//! This crate is intentionally independent of application state and feature
//! modules so every higher layer can consume the same validated settings,
//! client policy, and safe error representation without dependency cycles.

pub mod cli;
pub mod config;
pub mod error;
pub mod external_http;
