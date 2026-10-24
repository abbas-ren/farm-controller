//! Shared timeouts and identifiers for device HTTP handlers and edge controllers.
//!
//! These values mirror the Node `device-service` defaults used by
//! `DeviceActionQueueService`, `RelayService`, and test-cancellation paths.
//! Handlers and workers import them so edge HTTP calls and callback waits stay
//! consistent across the device API surface.

use std::time::Duration;

/// Numeric default used by device route tests that bind a stub edge listener.
/// Production call sites use `AppConfig.device.edge_*_http_port` instead.
#[cfg(test)]
pub(crate) const EDGE_CONTROLLER_PORT: u16 = 8888;

/// Hard-coded Gen3 controller id used when routing legacy Gen3 power actions.
pub(crate) const GEN3_CONTROLLER_ID: &str = "2ccf67bd0aef";

/// How long callback_store waits for a device action acknowledgement before failing.
pub(crate) const DEVICE_CALLBACK_TIMEOUT: Duration = Duration::from_secs(15);

/// How long relay identity remap waits for the controller callback.
pub(crate) const RELAY_IDENTITY_TIMEOUT: Duration = Duration::from_secs(10);

/// How long test-cancellation waits for the edge to acknowledge abort.
pub(crate) const TEST_CANCELLATION_TIMEOUT: Duration = Duration::from_secs(5);

/// HTTP client timeout for outbound device action requests to edge controllers.
pub(crate) const DEVICE_ACTION_HTTP_TIMEOUT: Duration = Duration::from_secs(55);

/// Brief pause between device command retries after a transient edge failure.
pub(crate) const DEVICE_COMMAND_RETRY_DELAY: Duration = Duration::from_millis(200);

/// How long relay configuration waits for the controller to confirm channel mapping.
pub(crate) const RELAY_CONTROLLER_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound for UART configuration round-trips with the edge controller.
pub(crate) const UART_CONFIGURATION_TIMEOUT: Duration = Duration::from_secs(300);

/// Hard cap on page size for list endpoints (alerts, builds, inventory, etc.).
pub(crate) const MAX_PAGE_SIZE: u32 = 100;
