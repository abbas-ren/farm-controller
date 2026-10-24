//! In-memory power/relay repository fake for device route tests.
//!
//! Stands in for production power toggle, relay config, and legacy relay CRUD methods.

use super::*;

#[async_trait]
impl PowerRelayRepository for RegistrationRepository {
    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::device_reboot_target`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn device_reboot_target(
        &self,
        _device_id: &str,
    ) -> Result<Option<DeviceRebootTarget>, DeviceRepositoryError> {
        Ok(None)
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::device_power_target`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn device_power_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError> {
        Ok((device_id == "power-device").then(|| DevicePowerTarget {
            device_family: Some("Gen4".to_owned()),
            current_power: Some("on".to_owned()),
            controller_ip: Some("127.0.0.1".to_owned()),
            power_port: None,
            relay_serial: Some("relay-1".to_owned()),
            channel_id: Some(Uuid::nil()),
            channel_number: Some(2),
        }))
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::apply_device_power`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn apply_device_power(
        &self,
        device_id: &str,
        channel_id: Option<Uuid>,
        state: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        if device_id != "power-device" {
            return Ok(None);
        }
        self.callbacks
            .lock()
            .unwrap()
            .push(format!("power:{device_id}:{state}"));
        Ok(Some(serde_json::json!({
            "id": channel_id,
            "deviceId": device_id,
            "channelNumber": 2,
            "state": state,
        })))
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::relays_for_controller`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn relays_for_controller(
        &self,
        _controller_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(Vec::new())
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::channels_for_relay`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn channels_for_relay(
        &self,
        _relay_id: Uuid,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
        Ok(Some(Vec::new()))
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::available_relay_devices`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn available_relay_devices(
        &self,
        query: &AvailableRelayDevicesQuery,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        self.callbacks.lock().unwrap().push(format!(
            "available:{}:{}:{}",
            query.page.unwrap_or(1),
            query.limit.unwrap_or(10),
            query
                .relay_id
                .map(|relay_id| relay_id.to_string())
                .unwrap_or_default()
        ));
        Ok(serde_json::json!({"devices": [{
            "deviceId": "device-1",
            "macAddress": "00:11:22:33:44:55",
            "deviceName": "Bench device",
            "ipAddress": "192.0.2.10",
            "deviceType": "Racer",
            "deviceFamily": "Gen4"
        }], "pagination": {
            "page": query.page.unwrap_or(1),
            "limit": query.limit.unwrap_or(10),
            "totalCount": 1,
            "totalPages": 1,
        }}))
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::relay_state_target`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn relay_state_target(
        &self,
        _device_id: &str,
    ) -> Result<Option<RelayStateTarget>, DeviceRepositoryError> {
        Ok(None)
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::update_relay_state`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn update_relay_state(
        &self,
        _channel_id: Uuid,
        _state: &str,
    ) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::relay_identity_target`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn relay_identity_target(
        &self,
        _relay_id: Uuid,
    ) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError> {
        Ok(Some(RelayIdentityTarget {
            controller_id: "controller-1".to_owned(),
            controller_ip: "127.0.0.1".to_owned(),
            current_serial: "relay-1".to_owned(),
        }))
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::update_relay_identity`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn update_relay_identity(
        &self,
        _relay_id: Uuid,
        _old_serial: &str,
        _serial_number: &str,
        _vendor_id: &str,
        _product_id: &str,
    ) -> Result<bool, DeviceRepositoryError> {
        Ok(true)
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::configure_relay_channels`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn configure_relay_channels(
        &self,
        configurations: &[RelayChannelConfiguration],
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        Ok(RelayConfigurationResult {
            channels: configurations
                .iter()
                .map(|configuration| {
                    serde_json::json!({
                        "id": configuration.channel_id,
                        "relayId": configuration.relay_id,
                        "deviceId": configuration.device_id,
                    })
                })
                .collect(),
            actions: Vec::new(),
        })
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::confirm_relay_configuration`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn confirm_relay_configuration(
        &self,
        mac: &str,
        succeeded: bool,
    ) -> Result<RelayConfirmationResponse, DeviceRepositoryError> {
        Ok(RelayConfirmationResponse {
            confirmed: succeeded,
            message: format!("Configuration callback for {mac}"),
            controller_id: Some("controller-1".to_owned()),
            relay_id: Some(Uuid::nil()),
            device_id: Some("device-1".to_owned()),
            configuration_complete: succeeded,
        })
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::legacy_relays`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn legacy_relays(
        &self,
        query: &LegacyRelayListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "id": Uuid::nil(),
            "deviceControllerId": query.controller_id,
            "serialNumber": "relay-1",
        })])
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::legacy_relay_channels`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn legacy_relay_channels(
        &self,
        query: &LegacyRelayChannelListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "id": Uuid::nil(),
            "relayId": query.relay_id,
            "deviceId": query.device_id,
            "channelNumber": 0,
        })])
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::legacy_relay_by_id`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn legacy_relay_by_id(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(Some(
            serde_json::json!({"id": relay_id, "serialNumber": "relay-1"}),
        ))
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::legacy_relay_channel_by_id`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn legacy_relay_channel_by_id(
        &self,
        channel_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(Some(
            serde_json::json!({"id": channel_id, "channelNumber": 0}),
        ))
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::delete_legacy_relay`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn delete_legacy_relay(&self, _relay_id: Uuid) -> Result<bool, DeviceRepositoryError> {
        Ok(true)
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::delete_legacy_relay_channel`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn delete_legacy_relay_channel(
        &self,
        _channel_id: Uuid,
    ) -> Result<bool, DeviceRepositoryError> {
        Ok(true)
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::configure_legacy_relay`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn configure_legacy_relay(
        &self,
        request: &LegacyRelayConfiguration,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        Ok(RelayConfigurationResult {
            channels: vec![serde_json::json!({
                "id": request.id,
                "relayId": request.relay_id,
                "deviceId": request.device_id,
            })],
            actions: Vec::new(),
        })
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::fresh_legacy_relay`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn fresh_legacy_relay(&self, _relay_id: Uuid) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::remap_legacy_relay`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn remap_legacy_relay(
        &self,
        _relay_id: Uuid,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        Ok(RelayConfigurationResult {
            channels: Vec::new(),
            actions: Vec::new(),
        })
    }

    /// PURPOSE: In-memory stand-in for `PowerRelayRepository::legacy_relay_conflicts`.
    /// USE: device route tests exercising power relay persistence without a live database.
    async fn legacy_relay_conflicts(
        &self,
        query: &LegacyRelayConflictQuery,
    ) -> Result<RelayConflictState, DeviceRepositoryError> {
        Ok(RelayConflictState {
            device_conflict: query.device_id.is_some(),
            relay_conflict: false,
            channel_conflict: query.channel_number == 1,
        })
    }
}
