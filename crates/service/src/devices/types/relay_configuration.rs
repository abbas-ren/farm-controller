use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::CallbackStatus;

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelayChannelConfiguration {
    pub relay_id: Uuid,
    pub channel_id: Uuid,
    pub device_id: Option<String>,
    pub gpio: Option<String>,
    #[serde(default = "default_low_voltage")]
    pub gpio_default_level: VoltageLevel,
    #[serde(default = "default_low_voltage")]
    pub relay_default_level: VoltageLevel,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum VoltageLevel {
    Low,
    High,
}

fn default_low_voltage() -> VoltageLevel {
    VoltageLevel::Low
}

#[cfg(test)]
mod voltage_default_tests {
    use super::{RelayChannelConfiguration, VoltageLevel};

    #[test]
    fn relay_channel_voltage_levels_default_low() {
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

#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum RelayConfigurationRequest {
    One(RelayChannelConfiguration),
    Many(Vec<RelayChannelConfiguration>),
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RelayConfigurationConfirmation {
    #[schema(example = "aabbccddeeff")]
    pub mac: String,
    pub status: CallbackStatus,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelayConfigurationAccepted {
    pub message: &'static str,
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelayConfirmationResponse {
    pub confirmed: bool,
    pub message: String,
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) controller_id: Option<String>,
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) relay_id: Option<Uuid>,
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) device_id: Option<String>,
    #[serde(skip)]
    #[schema(ignore)]
    pub(crate) configuration_complete: bool,
}
