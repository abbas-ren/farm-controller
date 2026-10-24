//! Apply and confirm modern relay channel ↔ device mappings.
//!
//! Ports Node `RelayService` configuration and `RelayConfigTimeoutService`
//! confirmation paths.

mod apply;
mod confirmation;
mod mappings;

pub(crate) use apply::configure;
pub(crate) use confirmation::confirm;
