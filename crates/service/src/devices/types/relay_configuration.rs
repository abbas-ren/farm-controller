//! Modern relay channel configuration request/response types.
//!
//! Wire shapes for admin relay mapping (`channelId` ↔ device/GPIO and default
//! voltage levels), ported from Node relay configuration APIs in
//! `farm_controller_old` device-service.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::CallbackStatus;

/// Request/response body type `RelayChannelConfiguration` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayChannelConfiguration {
    /// `RelayChannelConfiguration.relay_id` used by device HTTP handlers.
    pub relay_id: Uuid,
    /// `RelayChannelConfiguration.channel_id` used by device HTTP handlers.
    pub channel_id: Uuid,
    /// Stable device identifier (often MAC-derived).
    pub device_id: Option<String>,
    /// `RelayChannelConfiguration.gpio` used by device HTTP handlers.
    pub gpio: Option<String>,
    /// `RelayChannelConfiguration.gpio_default_level` used by device HTTP handlers.
    #[serde(default = "default_low_voltage")]
    pub gpio_default_level: VoltageLevel,
    /// `RelayChannelConfiguration.relay_default_level` used by device HTTP handlers.
    #[serde(default = "default_low_voltage")]
    pub relay_default_level: VoltageLevel,
}

/// Enumerated wire value `VoltageLevel` for device HTTP handlers.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum VoltageLevel {
    Low,
    High,
}

/// PURPOSE: Serde default returning VoltageLevel::Low for omitted level fields.
/// USE: Deserialization of relay configuration bodies.
fn default_low_voltage() -> VoltageLevel {
    let _api_trace = crate::observability::ApiTrace::enter("devices::types::relay_configuration", "default_low_voltage");
    VoltageLevel::Low
}

#[cfg(test)]
mod voltage_default_tests {
    use super::{RelayChannelConfiguration, VoltageLevel};

    #[test]
/// PURPOSE: Assert omitted gpio/relay default levels deserialize as Low.
/// USE: regression for relay configuration serde defaults.
    fn relay_channel_voltage_levels_default_low() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::types::relay_configuration", "relay_channel_voltage_levels_default_low");
        let configuration: RelayChannelConfiguration = serde_json::from_value(serde_json::json!({
            "relayId": uuid::Uuid::new_v4(),
            "channelId": uuid::Uuid::new_v4(),
            "deviceId": "device-1",
            "gpio": "17"
        }))
        .unwrap();

        assert_eq!(configuration.gpio_default_level, VoltageLevel::Low);
        assert_eq!(configuration.relay_default_level, VoltageLevel::Low);
    }
}

/// Enumerated wire value `RelayConfigurationRequest` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum RelayConfigurationRequest {
    One(RelayChannelConfiguration),
    Many(Vec<RelayChannelConfiguration>),
}

/// Request/response body type `RelayConfigurationConfirmation` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RelayConfigurationConfirmation {
    /// Device or interface MAC / UID identifier.
    #[schema(example = "aabbccddeeff")]
    pub mac: String,
    /// Outcome or lifecycle status for this record.
    pub status: CallbackStatus,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelayConfigurationAccepted {
    /// Human-readable status or error text for the client.
    pub message: &'static str,
    /// Payload body preserved for frontend compatibility.
    pub data: serde_json::Value,
}

/// Request/response body type `RelayConfirmationResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct RelayConfirmationResponse {
    /// Whether the operator confirmed the relay mapping.
    pub confirmed: bool,
    /// Human-readable status or error text for the client.
    pub message: String,
    /// EdgeController identifier.
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) controller_id: Option<String>,
    /// `RelayConfirmationResponse.relay_id` used by device HTTP handlers.
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) relay_id: Option<Uuid>,
    /// Stable device identifier (often MAC-derived).
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) device_id: Option<String>,
    /// `RelayConfirmationResponse.configuration_complete` used by device HTTP handlers.
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) configuration_complete: bool,
}
