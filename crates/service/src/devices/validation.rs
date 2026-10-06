use std::{collections::HashSet, net::Ipv4Addr, str::FromStr};

use super::{
    error::DeviceRepositoryError,
    types::{
        ControllerRegistration, DeviceRegistration, RelayChannelConfiguration,
        RelayChannelRegistration, RelayConfigurationRequest, RelayRegistration,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceInterfaceInput {
    pub interface_type: String,
    pub interface_id: String,
}

/// Validates a device registration and returns its normalized MAC identifier.
pub(crate) fn validate_device_registration(
    registration: &DeviceRegistration,
) -> Result<String, DeviceRepositoryError> {
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

/// Flattens grouped interface IDs into rows suitable for persistence.
pub(crate) fn device_interfaces(registration: &DeviceRegistration) -> Vec<DeviceInterfaceInput> {
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

/// Compares dotted firmware versions using the legacy upgrade direction.
pub(crate) fn compare_versions(current: &str, target: &str) -> i8 {
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

/// Validates single or batch relay assignments and reports whether input was batched.
pub(crate) fn relay_configurations(
    request: RelayConfigurationRequest,
) -> Result<(Vec<RelayChannelConfiguration>, bool), DeviceRepositoryError> {
    let (configurations, is_batch) = match request {
        RelayConfigurationRequest::One(configuration) => (vec![configuration], false),
        RelayConfigurationRequest::Many(configurations) => (configurations, true),
    };
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

/// Normalizes explicit or legacy state-map relay channels for persistence.
pub(crate) fn relay_channels(
    relay: &RelayRegistration,
) -> Result<Vec<RelayChannelRegistration>, DeviceRepositoryError> {
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

/// Validates the controller fields required before registration.
pub(crate) fn validate_registration(
    registration: &ControllerRegistration,
) -> Result<(), DeviceRepositoryError> {
    if registration.ip_address.trim().is_empty() {
        return Err(DeviceRepositoryError::Validation(
            "ipAddress must not be empty".to_owned(),
        ));
    }
    normalized_controller_id(&registration.mac_address).map(|_| ())
}

/// Removes supported MAC separators and validates a hexadecimal controller ID.
pub(crate) fn normalized_controller_id(mac_address: &str) -> Result<String, DeviceRepositoryError> {
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

/// Normalizes a mapping callback MAC with the controller identity rules.
pub(crate) fn normalize_mapping_mac(mac_address: &str) -> Result<String, DeviceRepositoryError> {
    normalized_controller_id(mac_address)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use uuid::Uuid;

    use crate::devices::VoltageLevel;

    use super::*;

    fn device_registration() -> DeviceRegistration {
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
    fn device_registration_normalizes_mac_and_accepts_ipv4() {
        assert_eq!(
            validate_device_registration(&device_registration()).unwrap(),
            "001a2b3c4d5e"
        );
    }

    #[test]
    fn device_registration_rejects_invalid_mac_ipv4_and_empty_name() {
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
    fn device_registration_flattens_interfaces_by_type() {
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
    fn firmware_version_comparison_matches_legacy_direction() {
        assert_eq!(compare_versions("v1.2.3", "1.3.0-beta"), 1);
        assert_eq!(compare_versions("2.0", "v1.9.9"), -1);
        assert_eq!(compare_versions("1.2", "1.2.0"), 0);
        assert_eq!(compare_versions("1.bad.3", "1.0.3"), 0);
    }

    fn configuration(channel_id: Uuid, device_id: Option<&str>) -> RelayChannelConfiguration {
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
    fn relay_configuration_rejects_duplicate_channels() {
        let channel_id = Uuid::new_v4();
        let error = relay_configurations(RelayConfigurationRequest::Many(vec![
            configuration(channel_id, Some("device-a")),
            configuration(channel_id, None),
        ]))
        .unwrap_err();
        assert!(error.to_string().contains("Duplicate channel IDs"));
    }

    #[test]
    fn relay_configuration_rejects_duplicate_non_empty_devices() {
        let error = relay_configurations(RelayConfigurationRequest::Many(vec![
            configuration(Uuid::new_v4(), Some("device-a")),
            configuration(Uuid::new_v4(), Some("device-a")),
        ]))
        .unwrap_err();
        assert!(error.to_string().contains("Duplicate device IDs"));
    }

    #[test]
    fn relay_configuration_allows_multiple_clears() {
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
