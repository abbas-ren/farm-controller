use super::super::error::DeviceRepositoryError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct InterfaceStatus {
    pub(super) interface_type: String,
    pub(super) interface_id: Option<String>,
    pub(super) status: &'static str,
}

#[derive(Clone, Debug)]
pub(super) struct InterfaceChange {
    pub(super) interface_type: String,
    pub(super) interface_id: String,
    pub(super) status: &'static str,
    pub(super) previous_status: Option<String>,
}

pub(super) async fn update_interface_status(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row_id: i32,
    status: &str,
) -> Result<(), DeviceRepositoryError> {
    sqlx::query(
        r#"UPDATE device_interfaces
           SET status = ($2::text)::"enum_device_interfaces_status", "updatedAt" = now()
           WHERE id = $1"#,
    )
    .bind(row_id)
    .bind(status)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

pub(super) fn interface_heartbeat_data(heartbeat: &serde_json::Value) -> serde_json::Value {
    let mut data = heartbeat.get("data").cloned().unwrap_or_default();
    if let Some(encoded) = data.get("data").and_then(serde_json::Value::as_str)
        && let Ok(decoded) = serde_json::from_str(encoded)
    {
        data = decoded;
    }
    data
}

pub(super) fn parse_interface_statuses(data: &serde_json::Value) -> Vec<InterfaceStatus> {
    let Some(categories) = data.as_object() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for (interface_type, interfaces) in categories {
        if interfaces.is_null()
            || interfaces
                .as_object()
                .is_some_and(|value| value.contains_key("not present"))
        {
            result.push(InterfaceStatus {
                interface_type: interface_type.clone(),
                interface_id: None,
                status: "not_available",
            });
            continue;
        }
        let Some(entries) = interfaces.as_object() else {
            continue;
        };
        for (interface_id, value) in entries {
            result.push(InterfaceStatus {
                interface_type: interface_type.clone(),
                interface_id: Some(interface_id.clone()),
                status: resolve_interface_status(value),
            });
        }
    }
    result
}

fn resolve_interface_status(data: &serde_json::Value) -> &'static str {
    let Some(entry) = data.as_object() else {
        return "unknown";
    };
    if let Some(slaves) = entry.get("slave").and_then(serde_json::Value::as_array) {
        return if slaves.is_empty() {
            "not_available"
        } else {
            "available"
        };
    }
    if entry
        .get("temp")
        .and_then(serde_json::Value::as_f64)
        .is_some()
    {
        return "available";
    }
    match entry
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("up") => "up",
        Some("down") => "down",
        Some("connected") => "connected",
        Some("disconnected") => "disconnected",
        _ => "unknown",
    }
}

pub(super) fn is_healthy_interface_status(status: &str) -> bool {
    matches!(status, "up" | "connected" | "available" | "plugged")
}

pub(super) fn interface_change_value(change: InterfaceChange) -> serde_json::Value {
    let mut value = serde_json::json!({
        "type": change.interface_type,
        "interfaceId": change.interface_id,
        "status": change.status,
    });
    if let Some(previous_status) = change.previous_status {
        value["previousStatus"] = serde_json::Value::String(previous_status);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_device_interface_heartbeat_shapes() {
        let heartbeat = serde_json::json!({
            "type": "heartbeat",
            "data": {
                "ethernet": {"eth0": {"status": "UP"}},
                "i2c": {"i2c-1": {"slave": ["0x20"]}},
                "temperature": {"cpu": {"temp": 41.5}},
                "usb": {"not present": " "},
                "serial": {"ttyS0": {"status": "disconnected"}}
            }
        });
        let data = interface_heartbeat_data(&heartbeat);
        assert_eq!(
            parse_interface_statuses(&data),
            vec![
                InterfaceStatus {
                    interface_type: "ethernet".to_owned(),
                    interface_id: Some("eth0".to_owned()),
                    status: "up"
                },
                InterfaceStatus {
                    interface_type: "i2c".to_owned(),
                    interface_id: Some("i2c-1".to_owned()),
                    status: "available"
                },
                InterfaceStatus {
                    interface_type: "serial".to_owned(),
                    interface_id: Some("ttyS0".to_owned()),
                    status: "disconnected"
                },
                InterfaceStatus {
                    interface_type: "temperature".to_owned(),
                    interface_id: Some("cpu".to_owned()),
                    status: "available"
                },
                InterfaceStatus {
                    interface_type: "usb".to_owned(),
                    interface_id: None,
                    status: "not_available"
                },
            ]
        );
    }

    #[test]
    fn decodes_nested_stringified_interface_data() {
        let heartbeat =
            serde_json::json!({"data": {"data": "{\"uart\":{\"ttyS1\":{\"status\":\"down\"}}}"}});
        assert_eq!(
            parse_interface_statuses(&interface_heartbeat_data(&heartbeat)),
            vec![InterfaceStatus {
                interface_type: "uart".to_owned(),
                interface_id: Some("ttyS1".to_owned()),
                status: "down",
            }]
        );
    }
}
