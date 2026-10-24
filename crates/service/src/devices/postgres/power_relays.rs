use super::*;

use crate::persistence::{
    entity::device,
    orm_util,
};
use sea_orm::{ColumnTrait, EntityTrait, FromQueryResult, QueryFilter};

#[derive(Debug, FromQueryResult)]
struct RebootTargetRow {
    controller_ip: Option<String>,
    power_port: Option<String>,
}

#[async_trait]
impl PowerRelayRepository for PostgresDeviceRepository {
    async fn device_reboot_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceRebootTarget>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "device_reboot_target",
        );
        let Some(device) = device::Entity::find()
            .filter(device::Column::DeviceId.eq(device_id))
            .filter(device::Column::DeletedAt.is_null())
            .one(self.orm())
            .await?
        else {
            return Ok(None);
        };

        // Lateral mapping lookup kept as SeaORM Statement (not sqlx) for JSON-key path.
        let mapping = orm_util::query_one::<_, RebootTargetRow>(
            self.orm(),
            r#"SELECT dc."ipAddress" AS controller_ip,
                      dc.mappings -> regexp_replace(lower($1), '[:-]', '', 'g') ->> 'power' AS power_port
               FROM device_controllers dc
               WHERE dc.state::text = 'active' AND dc.status::text = 'approved'
                 AND dc."deletedAt" IS NULL
                 AND dc.mappings ? regexp_replace(lower($1), '[:-]', '', 'g')
               ORDER BY dc."updatedAt" DESC LIMIT 1"#,
            vec![device.mac_address.clone().unwrap_or_default().into()],
        )
        .await?;

        Ok(Some(DeviceRebootTarget {
            device_family: device.device_family,
            controller_ip: mapping.as_ref().and_then(|m| m.controller_ip.clone()),
            power_port: mapping.and_then(|m| m.power_port),
        }))
    }

    async fn device_power_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "device_power_target",
        );
        relay_store::device_power_target(self.orm(), device_id).await
    }

    async fn apply_device_power(
        &self,
        device_id: &str,
        channel_id: Option<Uuid>,
        state: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "apply_device_power",
        );
        relay_store::apply_device_power(self.orm(), device_id, channel_id, state).await
    }

    async fn relays_for_controller(
        &self,
        controller_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "relays_for_controller",
        );
        relay_store::relays_for_controller(self.orm(), controller_id).await
    }

    async fn channels_for_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "channels_for_relay",
        );
        relay_store::channels_for_relay(self.orm(), relay_id).await
    }

    async fn available_relay_devices(
        &self,
        query: &AvailableRelayDevicesQuery,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "available_relay_devices",
        );
        relay_store::available_devices(self.orm(), query).await
    }

    async fn relay_state_target(
        &self,
        device_id: &str,
    ) -> Result<Option<RelayStateTarget>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "relay_state_target",
        );
        relay_store::state_target(self.orm(), device_id).await
    }

    async fn update_relay_state(
        &self,
        channel_id: Uuid,
        state: &str,
    ) -> Result<(), DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "update_relay_state",
        );
        relay_store::update_state(self.orm(), channel_id, state).await
    }

    async fn relay_identity_target(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "relay_identity_target",
        );
        relay_store::identity_target(self.orm(), relay_id).await
    }

    async fn update_relay_identity(
        &self,
        relay_id: Uuid,
        old_serial: &str,
        serial_number: &str,
        vendor_id: &str,
        product_id: &str,
    ) -> Result<bool, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "update_relay_identity",
        );
        relay_store::update_identity(
            self.orm(),
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
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "configure_relay_channels",
        );
        relay_configuration_store::configure(self.orm(), configurations).await
    }

    async fn confirm_relay_configuration(
        &self,
        mac: &str,
        succeeded: bool,
    ) -> Result<RelayConfirmationResponse, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "confirm_relay_configuration",
        );
        relay_configuration_store::confirm(self.orm(), mac, succeeded).await
    }

    async fn legacy_relays(
        &self,
        query: &LegacyRelayListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "legacy_relays",
        );
        legacy_relay_store::list_relays(self.orm(), query).await
    }

    async fn legacy_relay_channels(
        &self,
        query: &LegacyRelayChannelListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "legacy_relay_channels",
        );
        legacy_relay_store::list_channels(self.orm(), query).await
    }

    async fn legacy_relay_by_id(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "legacy_relay_by_id",
        );
        legacy_relay_store::relay_by_id(self.orm(), relay_id).await
    }

    async fn legacy_relay_channel_by_id(
        &self,
        channel_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "legacy_relay_channel_by_id",
        );
        legacy_relay_store::channel_by_id(self.orm(), channel_id).await
    }

    async fn delete_legacy_relay(&self, relay_id: Uuid) -> Result<bool, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "delete_legacy_relay",
        );
        legacy_relay_store::delete_relay(self.orm(), relay_id).await
    }

    async fn delete_legacy_relay_channel(
        &self,
        channel_id: Uuid,
    ) -> Result<bool, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "delete_legacy_relay_channel",
        );
        legacy_relay_store::delete_channel(self.orm(), channel_id).await
    }

    async fn configure_legacy_relay(
        &self,
        request: &LegacyRelayConfiguration,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "configure_legacy_relay",
        );
        legacy_relay_store::configure(self.orm(), request).await
    }

    async fn fresh_legacy_relay(&self, relay_id: Uuid) -> Result<(), DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "fresh_legacy_relay",
        );
        legacy_relay_store::fresh(self.orm(), relay_id).await
    }

    async fn remap_legacy_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "remap_legacy_relay",
        );
        legacy_relay_store::remap(self.orm(), relay_id).await
    }

    async fn legacy_relay_conflicts(
        &self,
        query: &LegacyRelayConflictQuery,
    ) -> Result<RelayConflictState, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::postgres::power_relays",
            "legacy_relay_conflicts",
        );
        legacy_relay_store::check_conflicts(self.orm(), query).await
    }
}
