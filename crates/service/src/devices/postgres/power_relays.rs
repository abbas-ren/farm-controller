use super::*;

#[async_trait]
impl PowerRelayRepository for PostgresDeviceRepository {
    async fn device_reboot_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceRebootTarget>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, DeviceRebootTarget>(
                        r#"SELECT d."deviceFamily" AS device_family,
                                 mapping."ipAddress" AS controller_ip,
                                 mapping.power_port
                             FROM devices d
                             LEFT JOIN LATERAL (
                                 SELECT dc."ipAddress",
                                     dc.mappings -> regexp_replace(lower(d."macAddress"), '[:-]', '', 'g') ->> 'power' AS power_port
                                 FROM device_controllers dc
                                 WHERE dc.state::text = 'active' AND dc.status::text = 'approved'
                                     AND dc."deletedAt" IS NULL
                                     AND dc.mappings ? regexp_replace(lower(d."macAddress"), '[:-]', '', 'g')
                                 ORDER BY dc."updatedAt" DESC LIMIT 1
                             ) mapping ON true
                             WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL"#,
                )
                .bind(device_id)
                .fetch_optional(&self.pool)
                .await?)
    }

    async fn device_power_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError> {
        relay_store::device_power_target(&self.pool, device_id).await
    }

    async fn apply_device_power(
        &self,
        device_id: &str,
        channel_id: Option<Uuid>,
        state: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        relay_store::apply_device_power(&self.pool, device_id, channel_id, state).await
    }

    async fn relays_for_controller(
        &self,
        controller_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        relay_store::relays_for_controller(&self.pool, controller_id).await
    }

    async fn channels_for_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
        relay_store::channels_for_relay(&self.pool, relay_id).await
    }

    async fn available_relay_devices(
        &self,
        query: &AvailableRelayDevicesQuery,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        relay_store::available_devices(&self.pool, query).await
    }

    async fn relay_state_target(
        &self,
        device_id: &str,
    ) -> Result<Option<RelayStateTarget>, DeviceRepositoryError> {
        relay_store::state_target(&self.pool, device_id).await
    }

    async fn update_relay_state(
        &self,
        channel_id: Uuid,
        state: &str,
    ) -> Result<(), DeviceRepositoryError> {
        relay_store::update_state(&self.pool, channel_id, state).await
    }

    async fn relay_identity_target(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError> {
        relay_store::identity_target(&self.pool, relay_id).await
    }

    async fn update_relay_identity(
        &self,
        relay_id: Uuid,
        old_serial: &str,
        serial_number: &str,
        vendor_id: &str,
        product_id: &str,
    ) -> Result<bool, DeviceRepositoryError> {
        relay_store::update_identity(
            &self.pool,
            relay_id,
            old_serial,
            serial_number,
            vendor_id,
            product_id,
        )
        .await
    }

    async fn configure_relay_channels(
        &self,
        configurations: &[RelayChannelConfiguration],
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        relay_configuration_store::configure(&self.pool, configurations).await
    }

    async fn confirm_relay_configuration(
        &self,
        mac: &str,
        succeeded: bool,
    ) -> Result<RelayConfirmationResponse, DeviceRepositoryError> {
        relay_configuration_store::confirm(&self.pool, mac, succeeded).await
    }

    async fn legacy_relays(
        &self,
        query: &LegacyRelayListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        legacy_relay_store::list_relays(&self.pool, query).await
    }

    async fn legacy_relay_channels(
        &self,
        query: &LegacyRelayChannelListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        legacy_relay_store::list_channels(&self.pool, query).await
    }

    async fn legacy_relay_by_id(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        legacy_relay_store::relay_by_id(&self.pool, relay_id).await
    }

    async fn legacy_relay_channel_by_id(
        &self,
        channel_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        legacy_relay_store::channel_by_id(&self.pool, channel_id).await
    }

    async fn delete_legacy_relay(&self, relay_id: Uuid) -> Result<bool, DeviceRepositoryError> {
        legacy_relay_store::delete_relay(&self.pool, relay_id).await
    }

    async fn delete_legacy_relay_channel(
        &self,
        channel_id: Uuid,
    ) -> Result<bool, DeviceRepositoryError> {
        legacy_relay_store::delete_channel(&self.pool, channel_id).await
    }

    async fn configure_legacy_relay(
        &self,
        request: &LegacyRelayConfiguration,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        legacy_relay_store::configure(&self.pool, request).await
    }

    async fn fresh_legacy_relay(&self, relay_id: Uuid) -> Result<(), DeviceRepositoryError> {
        legacy_relay_store::fresh(&self.pool, relay_id).await
    }

    async fn remap_legacy_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        legacy_relay_store::remap(&self.pool, relay_id).await
    }

    async fn legacy_relay_conflicts(
        &self,
        query: &LegacyRelayConflictQuery,
    ) -> Result<RelayConflictState, DeviceRepositoryError> {
        legacy_relay_store::check_conflicts(&self.pool, query).await
    }
}
