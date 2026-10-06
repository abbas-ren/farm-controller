use async_trait::async_trait;
use uuid::Uuid;

use super::*;

/// Owns device power operations and modern/legacy relay configuration.
#[async_trait]
pub(crate) trait PowerRelayRepository: Send + Sync {
    async fn device_reboot_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceRebootTarget>, DeviceRepositoryError>;
    async fn device_power_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError>;
    async fn apply_device_power(
        &self,
        device_id: &str,
        channel_id: Option<Uuid>,
        state: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn relays_for_controller(
        &self,
        controller_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn channels_for_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError>;
    async fn available_relay_devices(
        &self,
        query: &AvailableRelayDevicesQuery,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn relay_state_target(
        &self,
        device_id: &str,
    ) -> Result<Option<RelayStateTarget>, DeviceRepositoryError>;
    async fn update_relay_state(
        &self,
        channel_id: Uuid,
        state: &str,
    ) -> Result<(), DeviceRepositoryError>;
    async fn relay_identity_target(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError>;
    async fn update_relay_identity(
        &self,
        relay_id: Uuid,
        old_serial: &str,
        serial_number: &str,
        vendor_id: &str,
        product_id: &str,
    ) -> Result<bool, DeviceRepositoryError>;
    async fn configure_relay_channels(
        &self,
        configurations: &[RelayChannelConfiguration],
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError>;
    async fn confirm_relay_configuration(
        &self,
        mac: &str,
        succeeded: bool,
    ) -> Result<RelayConfirmationResponse, DeviceRepositoryError>;
    async fn legacy_relays(
        &self,
        query: &LegacyRelayListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn legacy_relay_channels(
        &self,
        query: &LegacyRelayChannelListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;
    async fn legacy_relay_by_id(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn legacy_relay_channel_by_id(
        &self,
        channel_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;
    async fn delete_legacy_relay(&self, relay_id: Uuid) -> Result<bool, DeviceRepositoryError>;
    async fn delete_legacy_relay_channel(
        &self,
        channel_id: Uuid,
    ) -> Result<bool, DeviceRepositoryError>;
    async fn configure_legacy_relay(
        &self,
        request: &LegacyRelayConfiguration,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError>;
    async fn fresh_legacy_relay(&self, relay_id: Uuid) -> Result<(), DeviceRepositoryError>;
    async fn remap_legacy_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError>;
    async fn legacy_relay_conflicts(
        &self,
        query: &LegacyRelayConflictQuery,
    ) -> Result<RelayConflictState, DeviceRepositoryError>;
}
