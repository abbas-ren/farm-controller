//! GPIO mapping JSON helpers for controller `mappings` columns.
//!
//! Shared by apply/confirm when updating Gen5-style GPIO maps.

use serde_json::{Map, Value};

use super::super::VoltageLevel;

/// PURPOSE: Normalize `normalized mapping key` (MAC/keys) for stable lookups.
/// USE: Registration, mapping, and heartbeat correlation paths.
fn normalized_mapping_key(value: &str) -> String {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_configuration_store::mappings", "normalized_mapping_key");
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

/// PURPOSE: Implement `matching_mapping_key` domain/persistence logic.
/// USE: Called as `matching_mapping_key` from handlers, repository impls, or sibling helpers.
fn matching_mapping_key(mappings: &Map<String, Value>, mac: &str) -> Option<String> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_configuration_store::mappings", "matching_mapping_key");
    let normalized_mac = normalized_mapping_key(mac);
    mappings
        .keys()
        .find(|key| normalized_mapping_key(key) == normalized_mac)
        .cloned()
}

/// PURPOSE: Implement `clear_gpio_mapping` domain/persistence logic.
/// USE: Called as `clear_gpio_mapping` from handlers, repository impls, or sibling helpers.
pub(super) fn clear_gpio_mapping(mappings: &mut Value, mac: &str) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_configuration_store::mappings", "clear_gpio_mapping");
    let Some(mappings) = mappings.as_object_mut() else {
        return;
    };
    let Some(key) = matching_mapping_key(mappings, mac) else {
        return;
    };
    let Some(entry) = mappings.get_mut(&key).and_then(Value::as_object_mut) else {
        return;
    };
    entry.remove("gpio");
    entry.remove("gpioDefaultLevel");
    entry.remove("relayDefaultLevel");
    entry.remove("gpioDownloadMode");
    entry.remove("relayDownloadMode");
    if entry.is_empty() {
        mappings.remove(&key);
    }
}

/// PURPOSE: Implement `upsert_gpio_mapping` domain/persistence logic.
/// USE: Called as `upsert_gpio_mapping` from handlers, repository impls, or sibling helpers.
pub(super) fn upsert_gpio_mapping(
    mappings: &mut Value,
    mac: &str,
    gpio: Option<&str>,
    gpio_default_level: VoltageLevel,
    relay_default_level: VoltageLevel,
) {
    let _api_trace = crate::observability::ApiTrace::enter("devices::relay_configuration_store::mappings", "upsert_gpio_mapping");
    let gpio = gpio.unwrap_or_default();
    if gpio.trim().is_empty() {
        clear_gpio_mapping(mappings, mac);
        return;
    }
    if !mappings.is_object() {
        *mappings = Value::Object(Map::new());
    }
    let mappings = mappings
        .as_object_mut()
        .expect("mapping object initialized");
    let key = matching_mapping_key(mappings, mac).unwrap_or_else(|| mac.to_owned());
    let entry = mappings
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    let entry = entry.as_object_mut().expect("mapping entry initialized");
    entry.remove("gpioDownloadMode");
    entry.remove("relayDownloadMode");
    entry.insert("gpio".to_owned(), Value::String(gpio.to_owned()));
    entry.insert(
        "gpioDefaultLevel".to_owned(),
        serde_json::to_value(gpio_default_level).expect("voltage level serializes"),
    );
    entry.insert(
        "relayDefaultLevel".to_owned(),
        serde_json::to_value(relay_default_level).expect("voltage level serializes"),
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::devices::VoltageLevel;

    use super::*;

    #[test]
    /// PURPOSE: Implement `gpio_updates_preserve_uart_and_power_mapping` domain/persistence logic.
    /// USE: Called as `gpio_updates_preserve_uart_and_power_mapping` from handlers, repository impls, or sibling helpers.
    fn gpio_updates_preserve_uart_and_power_mapping() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::relay_configuration_store::mappings", "gpio_updates_preserve_uart_and_power_mapping");
        let mut mappings = json!({"AA-BB": {
            "uart": "ttyUSB0",
            "power": "P1",
            "gpioDownloadMode": "HIGH",
            "relayDownloadMode": "HIGH"
        }});
        upsert_gpio_mapping(
            &mut mappings,
            "aa:bb",
            Some("4"),
            VoltageLevel::Low,
            VoltageLevel::Low,
        );
        assert_eq!(mappings["AA-BB"]["uart"], "ttyUSB0");
        assert_eq!(mappings["AA-BB"]["gpio"], "4");
        assert_eq!(mappings["AA-BB"]["gpioDefaultLevel"], "LOW");
        assert_eq!(mappings["AA-BB"]["relayDefaultLevel"], "LOW");
        assert!(mappings["AA-BB"].get("gpioDownloadMode").is_none());
        assert!(mappings["AA-BB"].get("relayDownloadMode").is_none());
        clear_gpio_mapping(&mut mappings, "aabb");
        assert_eq!(
            mappings,
            json!({"AA-BB": {"uart": "ttyUSB0", "power": "P1"}})
        );
    }
}
