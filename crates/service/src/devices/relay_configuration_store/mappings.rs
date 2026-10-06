use serde_json::{Map, Value};

use super::super::VoltageLevel;

fn normalized_mapping_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

fn matching_mapping_key(mappings: &Map<String, Value>, mac: &str) -> Option<String> {
    let normalized_mac = normalized_mapping_key(mac);
    mappings
        .keys()
        .find(|key| normalized_mapping_key(key) == normalized_mac)
        .cloned()
}

pub(super) fn clear_gpio_mapping(mappings: &mut Value, mac: &str) {
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

pub(super) fn upsert_gpio_mapping(
    mappings: &mut Value,
    mac: &str,
    gpio: Option<&str>,
    gpio_default_level: VoltageLevel,
    relay_default_level: VoltageLevel,
) {
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
    fn gpio_updates_preserve_uart_and_power_mapping() {
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
