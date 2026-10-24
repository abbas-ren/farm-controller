//! Power and relay configuration capability trait (Node `RelayService`).

use async_trait::async_trait;
use uuid::Uuid;

use super::*;

/// Owns device power operations and modern/legacy relay configuration.
#[async_trait]
pub(crate) trait PowerRelayRepository: Send + Sync {
    /// PURPOSE: Resolve edge/controller targets for `device reboot target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn device_reboot_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceRebootTarget>, DeviceRepositoryError>;
    /// PURPOSE: Resolve edge/controller targets for `device power target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn device_power_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError>;
    /// PURPOSE: Mutate persisted state for `apply device power`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn apply_device_power(
        &self,
        device_id: &str,
        channel_id: Option<Uuid>,
        state: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `relays_for_controller` domain/persistence logic.
    /// USE: Called as `relays_for_controller` from handlers, repository impls, or sibling helpers.
    async fn relays_for_controller(
        &self,
        controller_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `channels_for_relay` domain/persistence logic.
    /// USE: Called as `channels_for_relay` from handlers, repository impls, or sibling helpers.
    async fn channels_for_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError>;
    /// PURPOSE: Implement `available_relay_devices` domain/persistence logic.
    /// USE: Called as `available_relay_devices` from handlers, repository impls, or sibling helpers.
    async fn available_relay_devices(
        &self,
        query: &AvailableRelayDevicesQuery,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    /// PURPOSE: Resolve edge/controller targets for `relay state target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn relay_state_target(
        &self,
        device_id: &str,
    ) -> Result<Option<RelayStateTarget>, DeviceRepositoryError>;
    /// PURPOSE: Mutate persisted state for `update relay state`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn update_relay_state(
        &self,
        channel_id: Uuid,
        state: &str,
    ) -> Result<(), DeviceRepositoryError>;
    /// PURPOSE: Resolve edge/controller targets for `relay identity target`.
    /// USE: Handlers/workers before outbound edge HTTP.
    async fn relay_identity_target(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError>;
    /// PURPOSE: Mutate persisted state for `update relay identity`.
    /// USE: Update handlers and workers after side effects succeed.
    async fn update_relay_identity(
        &self,
        relay_id: Uuid,
        old_serial: &str,
        serial_number: &str,
        vendor_id: &str,
        product_id: &str,
    ) -> Result<bool, DeviceRepositoryError>;
    /// PURPOSE: Apply or confirm `configure relay channels` configuration.
    /// USE: Relay configuration handlers and timeout workers.
    async fn configure_relay_channels(
        &self,
        configurations: &[RelayChannelConfiguration],
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError>;
    /// PURPOSE: Apply or confirm `confirm relay configuration` configuration.
    /// USE: Relay configuration handlers and timeout workers.
    async fn confirm_relay_configuration(
        &self,
        mac: &str,
        succeeded: bool,
    ) -> Result<RelayConfirmationResponse, DeviceRepositoryError>;
    /// PURPOSE: Implement `legacy_relays` domain/persistence logic.
    /// USE: Called as `legacy_relays` from handlers, repository impls, or sibling helpers.
    async fn legacy_relays(
        &self,
        query: &LegacyRelayListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Implement `legacy_relay_channels` domain/persistence logic.
    /// USE: Called as `legacy_relay_channels` from handlers, repository impls, or sibling helpers.
    async fn legacy_relay_channels(
        &self,
        query: &LegacyRelayChannelListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Fetch a single record for `legacy relay by id`.
    /// USE: Detail handlers and repository lookups.
    async fn legacy_relay_by_id(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Fetch a single record for `legacy relay channel by id`.
    /// USE: Detail handlers and repository lookups.
    async fn legacy_relay_channel_by_id(
        &self,
        channel_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    /// PURPOSE: Delete `delete legacy relay` from persistence.
    /// USE: Delete handlers after edge cleanup when required.
    async fn delete_legacy_relay(&self, relay_id: Uuid) -> Result<bool, DeviceRepositoryError>;
    /// PURPOSE: Delete `delete legacy relay channel` from persistence.
    /// USE: Delete handlers after edge cleanup when required.
    async fn delete_legacy_relay_channel(
        &self,
        channel_id: Uuid,
    ) -> Result<bool, DeviceRepositoryError>;
    /// PURPOSE: Apply or confirm `configure legacy relay` configuration.
    /// USE: Relay configuration handlers and timeout workers.
    async fn configure_legacy_relay(
        &self,
        request: &LegacyRelayConfiguration,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError>;
    /// PURPOSE: Implement `fresh_legacy_relay` domain/persistence logic.
    /// USE: Called as `fresh_legacy_relay` from handlers, repository impls, or sibling helpers.
    async fn fresh_legacy_relay(&self, relay_id: Uuid) -> Result<(), DeviceRepositoryError>;
    /// PURPOSE: Implement `remap_legacy_relay` domain/persistence logic.
    /// USE: Called as `remap_legacy_relay` from handlers, repository impls, or sibling helpers.
    async fn remap_legacy_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError>;
    /// PURPOSE: Implement `legacy_relay_conflicts` domain/persistence logic.
    /// USE: Called as `legacy_relay_conflicts` from handlers, repository impls, or sibling helpers.
    async fn legacy_relay_conflicts(
        &self,
        query: &LegacyRelayConflictQuery,
    ) -> Result<RelayConflictState, DeviceRepositoryError>;
}
