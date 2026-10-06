use std::time::Duration;

pub(crate) const EDGE_CONTROLLER_PORT: u16 = 8888;
pub(crate) const GEN3_CONTROLLER_ID: &str = "2ccf67bd0aef";
pub(crate) const DEVICE_CALLBACK_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const RELAY_IDENTITY_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const TEST_CANCELLATION_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const DEVICE_ACTION_HTTP_TIMEOUT: Duration = Duration::from_secs(55);
pub(crate) const DEVICE_COMMAND_RETRY_DELAY: Duration = Duration::from_millis(200);
pub(crate) const RELAY_CONTROLLER_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const MAX_PAGE_SIZE: u32 = 100;
