//! HTTP handlers for test execution commands, queries, reports, and cancellation.
//! Re-exports preserve the historical `devices::test_execution_handlers` API.

mod cancellation;
mod case_queries;
mod commands;
mod events;
mod queries;
mod reports;
mod selection;
mod validation;

use crate::devices::*;

#[cfg(test)]
pub(crate) use cancellation::cancellation_events;
pub(crate) use cancellation::complete_deferred_cancellation;
pub use cancellation::*;
pub use case_queries::*;
pub(crate) use commands::create_for_user;
pub use commands::*;
#[cfg(test)]
pub(crate) use events::new_execution_event;
#[cfg(test)]
pub(super) use events::report_update_events;
pub use queries::*;
pub use reports::*;
