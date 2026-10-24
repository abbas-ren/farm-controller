//! Input validation and normalization for device / controller / relay registration.
//!
//! Shared by registration stores, relay configuration apply, and HTTP handlers
//! before persistence. Ports the Joi / custom validators from Node
//! `device-service/src/validations` and bits of `DeviceService` / `RelayService`
//! (MAC normalization, firmware compare direction, relay channel uniqueness).

use std::{collections::HashSet, net::Ipv4Addr, str::FromStr};

use super::{
    error::DeviceRepositoryError,
    types::{
        ControllerRegistration, DeviceRegistration, RelayChannelConfiguration,
        RelayChannelRegistration, RelayConfigurationRequest, RelayRegistration,
    },
};

/// Flattened interface row ready for `device_interfaces` insert.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceInterfaceInput {
    /// Interface class (ethernet, usb, …).
    pub interface_type: String,
    /// Interface identifier reported by the device (eth0, usb0, …).
    pub interface_id: String,
}

/// PURPOSE: Validate device registration fields and return a normalized MAC (no separators, lowercase).
/// USE: Device registration store / handler before insert or update.
pub(crate) fn validate_device_registration(
    registration: &DeviceRegistration,
) -> Result<String, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "validate_device_registration");
    let octets: Vec<&str> = registration.mac_address.split([':', '-']).collect();
    if octets.len() != 6
        || octets
            .iter()
            .any(|octet| octet.len() != 2 || !octet.chars().all(|value| value.is_ascii_hexdigit()))
    {
        return Err(DeviceRepositoryError::Validation(
            "Invalid MAC address format (e.g., 00:1A:2B:3C:4D:5E or 00-1a-2b-3c-4d-5e)".to_owned(),
        ));
    }
    if Ipv4Addr::from_str(&registration.ip_address).is_err() {
        return Err(DeviceRepositoryError::Validation(
            "Invalid IPv4 address".to_owned(),
        ));
    }
    if registration.device_name.is_empty() {
        return Err(DeviceRepositoryError::Validation(
            "Device name is required".to_owned(),
        ));
    }
    if !matches!(
        registration.state.as_str(),
        "busy" | "not_reachable" | "free" | "faulty" | "unknown"
    ) {
        return Err(DeviceRepositoryError::Validation(
            "Invalid device state".to_owned(),
        ));
    }
    if !matches!(
        registration.status.as_str(),
        "approved" | "requested" | "declined"
    ) {
        return Err(DeviceRepositoryError::Validation(
            "Invalid device status".to_owned(),
        ));
    }
    Ok(octets.concat().to_lowercase())
}

/// PURPOSE: Flatten grouped interface IDs into rows suitable for persistence.
/// USE: Device registration store when writing `device_interfaces`.
pub(crate) fn device_interfaces(registration: &DeviceRegistration) -> Vec<DeviceInterfaceInput> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "device_interfaces");
    registration
        .interfaces
        .iter()
        .flat_map(|(interface_type, interface_ids)| {
            interface_ids
                .iter()
                .map(|interface_id| DeviceInterfaceInput {
                    interface_type: interface_type.clone(),
                    interface_id: interface_id.clone(),
                })
        })
        .collect()
}

/// PURPOSE: Compare dotted firmware versions using the legacy upgrade direction.
/// USE: Flashing / upgrade paths decide whether `target` is newer (`1`), older (`-1`), or equal (`0`).
pub(crate) fn compare_versions(current: &str, target: &str) -> i8 {
    let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "compare_versions");
    // Strip leading v/V and pre-release suffix so `v1.2.3-beta` compares as `1.2.3`.
    let normalize = |version: &str| {
        version
            .trim()
            .trim_start_matches(['v', 'V'])
            .split('-')
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|part| part.parse::<i64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let current = normalize(current);
    let target = normalize(target);
    for index in 0..current.len().max(target.len()) {
        match target
            .get(index)
            .copied()
            .unwrap_or_default()
            .cmp(&current.get(index).copied().unwrap_or_default())
        {
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Equal => {}
        }
    }
    0
}

/// PURPOSE: Validate single or batch relay assignments and report whether input was batched.
/// USE: Relay configuration apply before persisting channel ↔ device mappings.
pub(crate) fn relay_configurations(
    request: RelayConfigurationRequest,
) -> Result<(Vec<RelayChannelConfiguration>, bool), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "relay_configurations");
    let (configurations, is_batch) = match request {
        RelayConfigurationRequest::One(configuration) => (vec![configuration], false),
        RelayConfigurationRequest::Many(configurations) => (configurations, true),
    };
    // Uniqueness mirrors Node RelayService: one channel id and one non-empty device per batch.
    let mut channels = HashSet::new();
    let mut devices = HashSet::new();
    for configuration in &configurations {
        if !channels.insert(configuration.channel_id) {
            return Err(DeviceRepositoryError::Validation(format!(
                "Duplicate channel IDs found: {}",
                configuration.channel_id
            )));
        }
        if let Some(device_id) = configuration
            .device_id
            .as_deref()
            .map(str::trim)
            .filter(|device_id| !device_id.is_empty())
            && !devices.insert(device_id.to_owned())
        {
            return Err(DeviceRepositoryError::Validation(format!(
                "Duplicate device IDs found: {device_id}"
            )));
        }
        if configuration
            .device_id
            .as_deref()
            .is_some_and(|device_id| !device_id.trim().is_empty())
            && configuration
                .gpio
                .as_deref()
                .map(str::trim)
                .filter(|gpio| !gpio.is_empty())
                .and_then(|gpio| gpio.parse::<u32>().ok())
                .is_none()
        {
            return Err(DeviceRepositoryError::Validation(
                "Assigned relay channels require one numeric GPIO".to_owned(),
            ));
        }
    }
    Ok((configurations, is_batch))
}

/// PURPOSE: Normalize explicit or legacy state-map relay channels for persistence.
/// USE: Relay registration store when channels array is empty and `state.channel_N` is used.
pub(crate) fn relay_channels(
    relay: &RelayRegistration,
) -> Result<Vec<RelayChannelRegistration>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "relay_channels");
    if !relay.channels.is_empty() {
        if relay
            .channels
            .iter()
            .any(|channel| channel.channel_number < 1)
        {
            return Err(DeviceRepositoryError::Validation(
                "relay channelNumber must be at least 1".to_owned(),
            ));
        }
        return Ok(relay
            .channels
            .iter()
            .map(|channel| RelayChannelRegistration {
                relay_channel_id: channel.relay_channel_id,
                channel_number: channel.channel_number,
                device_id: channel.device_id.clone(),
            })
            .collect());
    }

    relay
        .state
        .keys()
        .filter_map(|key| key.strip_prefix("channel_").map(str::parse::<i32>))
        .map(|channel| {
            channel
                .map(|channel_number| RelayChannelRegistration {
                    relay_channel_id: None,
                    channel_number,
                    device_id: None,
                })
                .map_err(|_| {
                    DeviceRepositoryError::Validation(
                        "relay state channel keys must end in an integer".to_owned(),
                    )
                })
        })
        .collect()
}

/// PURPOSE: Validate the controller fields required before registration.
/// USE: Controller registration store / handler entry.
pub(crate) fn validate_registration(
    registration: &ControllerRegistration,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "validate_registration");
    if registration.ip_address.trim().is_empty() {
        return Err(DeviceRepositoryError::Validation(
            "ipAddress must not be empty".to_owned(),
        ));
    }
    normalized_controller_id(&registration.mac_address).map(|_| ())
}

/// PURPOSE: Remove supported MAC separators and validate a hexadecimal controller ID.
/// USE: Controller registration and any path that keys controllers by MAC-derived id.
pub(crate) fn normalized_controller_id(mac_address: &str) -> Result<String, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "normalized_controller_id");
    let controller_id = mac_address.trim().replace([':', '-'], "").to_lowercase();
    if controller_id.is_empty()
        || !controller_id
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(DeviceRepositoryError::Validation(
            "macAddress must contain hexadecimal characters".to_owned(),
        ));
    }
    Ok(controller_id)
}

/// PURPOSE: Normalize a Gen5 mapping callback MAC with the controller identity rules.
/// USE: Mapping callback / Gen5 mapping worker when correlating device MACs.
pub(crate) fn normalize_mapping_mac(mac_address: &str) -> Result<String, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "normalize_mapping_mac");
    normalized_controller_id(mac_address)
}

#[cfg(test)]
mod tests {
    //! Unit tests for MAC normalization, interface flattening, and relay uniqueness.

    use std::collections::BTreeMap;

    use uuid::Uuid;

    use crate::devices::VoltageLevel;

    use super::*;

/// PURPOSE: Build a minimal valid DeviceRegistration fixture for unit tests.
/// USE: validation module tests.
    fn device_registration() -> DeviceRegistration {
        let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "device_registration");
        DeviceRegistration {
            mac_address: "00:1A:2b:3C:4d:5E".to_owned(),
            ip_address: "192.168.1.20".to_owned(),
            device_name: "Bench device".to_owned(),
            device_family: Some("Gen5".to_owned()),
            device_type: Some("x5h".to_owned()),
            build_id: None,
            timeout: 5,
            interfaces: BTreeMap::new(),
            software_version: None,
            nfs_path: None,
            state: "free".to_owned(),
            status: "requested".to_owned(),
        }
    }

    #[test]
/// PURPOSE: Assert MAC separator stripping and IPv4 acceptance on registration.
/// USE: regression for Node DeviceService MAC normalization.
    fn device_registration_normalizes_mac_and_accepts_ipv4() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "device_registration_normalizes_mac_and_accepts_ipv4");
        assert_eq!(
            validate_device_registration(&device_registration()).unwrap(),
            "001a2b3c4d5e"
        );
    }

    #[test]
/// PURPOSE: Assert rejection of compacted MAC, IPv6, and empty device names.
/// USE: regression for registration validation edge cases.
    fn device_registration_rejects_invalid_mac_ipv4_and_empty_name() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "device_registration_rejects_invalid_mac_ipv4_and_empty_name");
        let mut registration = device_registration();
        registration.mac_address = "001a2b3c4d5e".to_owned();
        assert!(validate_device_registration(&registration).is_err());

        registration.mac_address = "00-1a-2b-3c-4d-5e".to_owned();
        registration.ip_address = "2001:db8::1".to_owned();
        assert!(validate_device_registration(&registration).is_err());

        registration.ip_address = "192.0.2.1".to_owned();
        registration.device_name.clear();
        assert!(validate_device_registration(&registration).is_err());
    }

    #[test]
/// PURPOSE: Assert interface map flattens into typed DeviceInterfaceInput rows.
/// USE: regression for device_interfaces persistence shaping.
    fn device_registration_flattens_interfaces_by_type() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "device_registration_flattens_interfaces_by_type");
        let mut registration = device_registration();
        registration.interfaces.insert(
            "ethernet".to_owned(),
            vec!["eth0".to_owned(), "eth1".to_owned()],
        );
        registration
            .interfaces
            .insert("usb".to_owned(), vec!["usb0".to_owned()]);

        assert_eq!(
            device_interfaces(&registration),
            vec![
                DeviceInterfaceInput {
                    interface_type: "ethernet".to_owned(),
                    interface_id: "eth0".to_owned(),
                },
                DeviceInterfaceInput {
                    interface_type: "ethernet".to_owned(),
                    interface_id: "eth1".to_owned(),
                },
                DeviceInterfaceInput {
                    interface_type: "usb".to_owned(),
                    interface_id: "usb0".to_owned(),
                },
            ]
        );
    }

    #[test]
/// PURPOSE: Assert compare_versions matches Node upgrade direction (target vs current).
/// USE: regression for flashing/upgrade version gates.
    fn firmware_version_comparison_matches_legacy_direction() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "firmware_version_comparison_matches_legacy_direction");
        assert_eq!(compare_versions("v1.2.3", "1.3.0-beta"), 1);
        assert_eq!(compare_versions("2.0", "v1.9.9"), -1);
        assert_eq!(compare_versions("1.2", "1.2.0"), 0);
        assert_eq!(compare_versions("1.bad.3", "1.0.3"), 0);
    }

/// PURPOSE: Build a RelayChannelConfiguration fixture with optional device/GPIO.
/// USE: relay configuration uniqueness tests.
    fn configuration(channel_id: Uuid, device_id: Option<&str>) -> RelayChannelConfiguration {
        let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "configuration");
        let gpio = device_id
            .is_some_and(|device_id| !device_id.trim().is_empty())
            .then(|| "17".to_owned());
        RelayChannelConfiguration {
            relay_id: Uuid::new_v4(),
            channel_id,
            device_id: device_id.map(str::to_owned),
            gpio,
            gpio_default_level: VoltageLevel::Low,
            relay_default_level: VoltageLevel::Low,
        }
    }

    #[test]
/// PURPOSE: Assert batch relay config rejects duplicate channel IDs.
/// USE: regression for RelayService channel uniqueness.
    fn relay_configuration_rejects_duplicate_channels() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "relay_configuration_rejects_duplicate_channels");
        let channel_id = Uuid::new_v4();
        let error = relay_configurations(RelayConfigurationRequest::Many(vec![
            configuration(channel_id, Some("device-a")),
            configuration(channel_id, None),
        ]))
        .unwrap_err();
        assert!(error.to_string().contains("Duplicate channel IDs"));
    }

    #[test]
/// PURPOSE: Assert batch relay config rejects the same non-empty device twice.
/// USE: regression for RelayService device uniqueness.
    fn relay_configuration_rejects_duplicate_non_empty_devices() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "relay_configuration_rejects_duplicate_non_empty_devices");
        let error = relay_configurations(RelayConfigurationRequest::Many(vec![
            configuration(Uuid::new_v4(), Some("device-a")),
            configuration(Uuid::new_v4(), Some("device-a")),
        ]))
        .unwrap_err();
        assert!(error.to_string().contains("Duplicate device IDs"));
    }

    #[test]
/// PURPOSE: Assert empty/whitespace device_id clears may repeat across channels.
/// USE: regression for unassigning multiple relay channels in one batch.
    fn relay_configuration_allows_multiple_clears() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::validation", "relay_configuration_allows_multiple_clears");
        let (configurations, is_batch) =
            relay_configurations(RelayConfigurationRequest::Many(vec![
                configuration(Uuid::new_v4(), None),
                configuration(Uuid::new_v4(), Some("  ")),
            ]))
            .unwrap();
        assert!(is_batch);
        assert_eq!(configurations.len(), 2);
    }
}
