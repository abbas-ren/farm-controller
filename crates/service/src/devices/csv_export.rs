//! Format device JSON rows into a CSV string for download.
//!
//! Consumes the payloads from [`device_export_store`] and emits columns that
//! match the Node `DeviceService` CSV export (fixed device fields plus dynamic
//! `{type}_interfaces` columns).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use super::error::DeviceRepositoryError;

/// Fixed leading CSV columns (device identity / lifecycle fields).
const BASE_HEADERS: [&str; 12] = [
    "deviceId",
    "deviceName",
    "deviceType",
    "macAddress",
    "ipAddress",
    "status",
    "state",
    "createdAt",
    "updatedAt",
    "stateUpdatedAt",
    "totalInterfaces",
    "lastConnectedOn",
];

/// PURPOSE: Render devices (+ nested interfaces) as a CSV document.
/// USE: Device export handler after loading rows from `device_export_store`.
pub(crate) fn format_devices(
    devices: &[serde_json::Value],
) -> Result<String, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::csv_export", "format_devices");
    // Dynamic columns: one per distinct interface type across the export set.
    let interface_types = devices
        .iter()
        .flat_map(interfaces)
        .filter_map(|interface| interface.get("type").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .write_record(
            BASE_HEADERS.into_iter().map(str::to_owned).chain(
                interface_types
                    .iter()
                    .map(|value| format!("{value}_interfaces")),
            ),
        )
        .map_err(csv_error)?;

    for device in devices {
        let mut grouped_interfaces = BTreeMap::<&str, Vec<&str>>::new();
        for interface in interfaces(device) {
            if let (Some(interface_type), Some(interface_id)) = (
                interface.get("type").and_then(serde_json::Value::as_str),
                interface
                    .get("interfaceId")
                    .and_then(serde_json::Value::as_str),
            ) {
                grouped_interfaces
                    .entry(interface_type)
                    .or_default()
                    .push(interface_id);
            }
        }
        let row = [
            text(device, "deviceId"),
            text(device, "deviceName"),
            text(device, "deviceType"),
            text(device, "macAddress"),
            text(device, "ipAddress"),
            text(device, "status"),
            text(device, "state"),
            date(device, "createdAt"),
            date(device, "updatedAt"),
            date(device, "stateUpdatedAt"),
            interfaces(device).len().to_string(),
            date(device, "lastConnectedOn"),
        ];
        writer
            .write_record(
                row.into_iter()
                    .chain(interface_types.iter().map(|interface_type| {
                        grouped_interfaces
                            .get(interface_type.as_str())
                            .map(|values| values.join(", "))
                            .unwrap_or_default()
                    })),
            )
            .map_err(csv_error)?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|error| csv_error(error.into_error().into()))?;
    String::from_utf8(bytes)
        .map_err(|error| DeviceRepositoryError::Validation(format!("Invalid CSV output: {error}")))
}

/// PURPOSE: Read the `interfaces` array from a device JSON object.
/// USE: Column discovery and per-row grouping inside `format_devices`.
fn interfaces(device: &serde_json::Value) -> &[serde_json::Value] {
    let _api_trace = crate::observability::ApiTrace::enter("devices::csv_export", "interfaces");
    device
        .get("interfaces")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// PURPOSE: Extract a string field or empty string.
/// USE: Fixed device columns in the CSV row.
fn text(device: &serde_json::Value, field: &str) -> String {
    let _api_trace = crate::observability::ApiTrace::enter("devices::csv_export", "text");
    device
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// PURPOSE: Format an RFC3339 timestamp as the legacy `Fri Oct 02 2026` date string.
/// USE: Date columns in the CSV export.
fn date(device: &serde_json::Value, field: &str) -> String {
    let _api_trace = crate::observability::ApiTrace::enter("devices::csv_export", "date");
    device
        .get(field)
        .and_then(serde_json::Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc).format("%a %b %d %Y").to_string())
        .unwrap_or_default()
}

/// PURPOSE: Wrap csv crate errors as validation failures for the HTTP layer.
/// USE: All writer fallible calls in this module.
fn csv_error(error: csv::Error) -> DeviceRepositoryError {
    let _api_trace = crate::observability::ApiTrace::enter("devices::csv_export", "csv_error");
    DeviceRepositoryError::Validation(format!("Failed to generate CSV: {error}"))
}

#[cfg(test)]
mod tests {
    //! Unit coverage for dynamic interface columns and date formatting.

    use super::*;

    #[test]
/// PURPOSE: Assert CSV gains dynamic interface columns and quotes joined IDs.
/// USE: regression for Node DeviceService CSV export shape.
    fn export_adds_dynamic_interface_columns_and_quotes_lists() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::csv_export", "export_adds_dynamic_interface_columns_and_quotes_lists");
        let csv = format_devices(&[serde_json::json!({
            "deviceId": "device-1",
            "deviceName": "Bench 1",
            "deviceType": "x5h",
            "macAddress": "00:11:22:33:44:55",
            "ipAddress": "192.0.2.10",
            "status": "approved",
            "state": "free",
            "createdAt": "2026-10-02T12:00:00Z",
            "updatedAt": "2026-10-03T12:00:00Z",
            "stateUpdatedAt": null,
            "lastConnectedOn": "2026-10-03T12:00:00Z",
            "interfaces": [
                {"type": "ethernet", "interfaceId": "eth0"},
                {"type": "ethernet", "interfaceId": "eth1"},
                {"type": "usb", "interfaceId": "usb0"}
            ]
        })])
        .unwrap();

        assert!(csv.starts_with("deviceId,deviceName,deviceType,"));
        assert!(csv.contains("ethernet_interfaces,usb_interfaces"));
        assert!(csv.contains("\"eth0, eth1\",usb0"));
        assert!(csv.contains("Fri Oct 02 2026"));
    }
}
