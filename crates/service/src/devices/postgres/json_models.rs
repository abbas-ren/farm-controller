//! Assemble API JSON from SeaORM entity models (replacing to_jsonb / jsonb_agg in SQL).

use crate::persistence::entity::{
    alert, device, device_controller, device_interface, faulty_report, relay, relay_channel,
    release, test_case, test_execution,
};

/// PURPOSE: Serialize `device status str` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn device_status_str(status: Option<&device::DeviceStatus>) -> Option<&'static str> {
    status.map(|s| match s {
        device::DeviceStatus::Approved => "approved",
        device::DeviceStatus::Requested => "requested",
        device::DeviceStatus::Declined => "declined",
    })
}

/// PURPOSE: Serialize `device state str` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn device_state_str(state: Option<&device::DeviceState>) -> Option<&'static str> {
    state.map(|s| match s {
        device::DeviceState::Busy => "busy",
        device::DeviceState::NotReachable => "not_reachable",
        device::DeviceState::Free => "free",
        device::DeviceState::Faulty => "faulty",
        device::DeviceState::Unknown => "unknown",
    })
}

/// PURPOSE: Serialize `device power str` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn device_power_str(power: Option<&device::DevicePower>) -> Option<&'static str> {
    power.map(|p| match p {
        device::DevicePower::On => "on",
        device::DevicePower::Off => "off",
    })
}

/// PURPOSE: Serialize `last execution status str` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn last_execution_status_str(
    status: Option<&device::TestStatus>,
) -> Option<&'static str> {
    status.map(|s| match s {
        device::TestStatus::NotExecuted => "not_executed",
        device::TestStatus::InProgress => "in_progress",
        device::TestStatus::Completed => "completed",
        device::TestStatus::Queued => "queued",
        device::TestStatus::Cancelled => "cancelled",
        device::TestStatus::Failed => "failed",
    })
}

/// PURPOSE: Serialize `device json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn device_json(model: &device::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "deviceName": model.device_name,
        "deviceFamily": model.device_family,
        "deviceId": model.device_id,
        "macAddress": model.mac_address,
        "ipAddress": model.ip_address,
        "deviceType": model.device_type,
        "state": device_state_str(model.state.as_ref()),
        "status": device_status_str(model.status.as_ref()),
        "lastConnectedOn": model.last_connected_on,
        "controllerId": model.controller_id,
        "createdBy": model.created_by,
        "updatedBy": model.updated_by,
        "statusActionBy": model.status_action_by,
        "lastTestExecution": model.last_test_execution,
        "lastExecutionStatus": last_execution_status_str(model.last_execution_status.as_ref()),
        "stateUpdatedAt": model.state_updated_at,
        "buildId": model.build_id,
        "heartbeatTimer": model.heartbeat_timer,
        "upgrading": model.upgrading,
        "flashing": model.flashing,
        "softwareVersion": model.software_version,
        "power": device_power_str(model.power.as_ref()),
        "nfsPath": model.nfs_path,
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
        "deletedAt": model.deleted_at,
    })
}

/// PURPOSE: Serialize `interface json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn interface_json(model: &device_interface::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "interfaceId": model.interface_id,
        "interfaceType": model.interface_type,
        "deviceId": model.device_id,
        "name": model.name,
        "type": model.r#type,
        "status": model.status.as_ref().map(|s| match s {
            device_interface::DeviceInterfaceStatus::Up => "up",
            device_interface::DeviceInterfaceStatus::Down => "down",
            device_interface::DeviceInterfaceStatus::Plugged => "plugged",
            device_interface::DeviceInterfaceStatus::Unplugged => "unplugged",
            device_interface::DeviceInterfaceStatus::Connected => "connected",
            device_interface::DeviceInterfaceStatus::Disconnected => "disconnected",
            device_interface::DeviceInterfaceStatus::Unknown => "unknown",
            device_interface::DeviceInterfaceStatus::Available => "available",
            device_interface::DeviceInterfaceStatus::NotAvailable => "not_available",
        }),
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
    })
}

/// PURPOSE: Serialize `alert json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn alert_json(model: &alert::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "userId": model.user_id,
        "title": model.title,
        "message": model.message,
        "data": model.data,
        "type": match model.r#type {
            alert::AlertType::Info => "info",
            alert::AlertType::Success => "success",
            alert::AlertType::Warning => "warning",
            alert::AlertType::Error => "error",
        },
        "status": match model.status {
            alert::AlertStatus::Unread => "unread",
            alert::AlertStatus::Read => "read",
        },
        "isRead": model.is_read,
        "readAt": model.read_at,
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
        "buildId": model.build_id,
        "faultyReportId": model.faulty_report_id,
        "deviceId": model.device_id,
        "deviceControllerId": model.device_controller_id,
    })
}

/// PURPOSE: Serialize `faulty report alert json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn faulty_report_alert_json(
    model: &alert::Model,
    build_version: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "userId": model.user_id,
        "title": model.title,
        "message": model.message,
        "type": match model.r#type {
            alert::AlertType::Info => "info",
            alert::AlertType::Success => "success",
            alert::AlertType::Warning => "warning",
            alert::AlertType::Error => "error",
        },
        "status": match model.status {
            alert::AlertStatus::Unread => "unread",
            alert::AlertStatus::Read => "read",
        },
        "isRead": model.is_read,
        "faultyReportId": model.faulty_report_id,
        "buildId": model.build_id,
        "buildVersion": build_version
            .map(str::to_owned)
            .or_else(|| {
                model
                    .data
                    .as_ref()
                    .and_then(|d| d.get("buildVersion"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            }),
        "data": model.data,
        "createdAt": model.created_at,
    })
}

/// PURPOSE: Serialize `faulty report json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn faulty_report_json(model: &faulty_report::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "releaseId": model.release_id,
        "deviceType": model.device_type,
        "deviceFamily": model.device_family,
        "lastTestExecutionId": model.last_test_execution_id,
        "description": model.description,
        "filePath": model.file_path,
        "logsPath": model.logs_path,
        "status": match model.status {
            faulty_report::FaultyStatus::Pending => "pending",
            faulty_report::FaultyStatus::Approved => "approved",
            faulty_report::FaultyStatus::Rejected => "rejected",
        },
        "createdBy": model.created_by,
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
    })
}

/// PURPOSE: Serialize `release json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn release_json(model: &release::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "folderName": model.folder_name,
        "version": model.version,
        "artifacts": model.artifacts,
        "lastScannedAt": model.last_scanned_at,
        "testedDeviceId": model.tested_device_id,
        "buildType": match model.build_type {
            release::BuildType::Custom => "custom",
            release::BuildType::Official => "official",
        },
        "allDevicesFlashed": model.all_devices_flashed,
        "uploadedBy": model.uploaded_by,
        "tag": model.tag,
        "status": model.status.as_ref().map(|s| match s {
            release::ReleaseTestStatus::Untested => "untested",
            release::ReleaseTestStatus::Testing => "testing",
            release::ReleaseTestStatus::Passed => "passed",
            release::ReleaseTestStatus::Failed => "failed",
        }),
        "isFaulty": model.is_faulty,
        "deviceFamily": model.device_family,
        "deviceType": model.device_type,
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
    })
}

/// PURPOSE: Serialize `controller json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn controller_json(model: &device_controller::Model) -> serde_json::Value {
    serde_json::json!({
        "deviceControllerId": model.device_controller_id,
        "macAddress": model.mac_address,
        "ipAddress": model.ip_address,
        "name": model.name,
        "deviceFamily": model.device_family,
        "mappings": model.mappings,
        "status": match model.status {
            device_controller::DeviceControllerStatus::Approved => "approved",
            device_controller::DeviceControllerStatus::Requested => "requested",
        },
        "state": match model.state {
            device_controller::DeviceControllerState::NotReachable => "not-reachable",
            device_controller::DeviceControllerState::Active => "active",
        },
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
        "deletedAt": model.deleted_at,
    })
}

/// PURPOSE: Serialize `relay json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn relay_json(model: &relay::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "serialNumber": model.serial_number,
        "vendorId": model.vendor_id,
        "productId": model.product_id,
        "deviceControllerId": model.device_controller_id,
        "state": match model.state {
            relay::RelayState::Connected => "connected",
            relay::RelayState::Disconnected => "disconnected",
        },
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
        "deletedAt": model.deleted_at,
    })
}

/// PURPOSE: Serialize `relay channel json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn relay_channel_json(model: &relay_channel::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "relayId": model.relay_id,
        "channelNumber": model.channel_number,
        "deviceId": model.device_id,
        "state": model.state.as_ref().map(|s| match s {
            relay_channel::ChannelState::On => "on",
            relay_channel::ChannelState::Off => "off",
        }),
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
        "deletedAt": model.deleted_at,
    })
}

/// Nested device summary attached to relay channel list responses.
pub(crate) fn relay_channel_device_summary(model: &device::Model) -> serde_json::Value {
    serde_json::json!({
        "deviceId": model.device_id,
        "macAddress": model.mac_address,
        "deviceName": model.device_name,
        "ipAddress": model.ip_address,
        "deviceType": model.device_type,
    })
}

/// PURPOSE: Serialize `relay channel with device json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn relay_channel_with_device_json(
    channel: &relay_channel::Model,
    device: Option<&device::Model>,
) -> serde_json::Value {
    let mut value = relay_channel_json(channel);
    value["device"] = device
        .map(relay_channel_device_summary)
        .unwrap_or(serde_json::Value::Null);
    value
}

/// Compact device row for available-relay-device listing.
pub(crate) fn available_relay_device_json(model: &device::Model) -> serde_json::Value {
    serde_json::json!({
        "deviceId": model.device_id,
        "macAddress": model.mac_address,
        "deviceName": model.device_name,
        "ipAddress": model.ip_address,
        "deviceType": model.device_type,
        "deviceFamily": model.device_family,
    })
}

/// Strip `:` / `-` and lowercase — matches SQL `regexp_replace(lower(...), '[:-]', '', 'g')`.
pub(crate) fn normalize_mapping_lookup_key(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|c| *c != ':' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Find a mappings entry by normalized MAC key (exact key, then fuzzy like jsonb_each).
pub(crate) fn mapping_entry_for_mac<'a>(
    mappings: &'a serde_json::Value,
    mac: &str,
) -> Option<&'a serde_json::Value> {
    let object = mappings.as_object()?;
    let normalized = normalize_mapping_lookup_key(mac);
    if normalized.is_empty() {
        return None;
    }
    if let Some(entry) = object.get(&normalized) {
        return Some(entry);
    }
    object.iter().find_map(|(key, value)| {
        (normalize_mapping_lookup_key(key) == normalized).then_some(value)
    })
}

/// PURPOSE: Implement `mapping_power_port` domain/persistence logic.
/// USE: Called as `mapping_power_port` from handlers, repository impls, or sibling helpers.
pub(crate) fn mapping_power_port(mappings: &serde_json::Value, mac: &str) -> Option<String> {
    mapping_entry_for_mac(mappings, mac)?
        .get("power")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// PURPOSE: Implement `mapping_uart_port` domain/persistence logic.
/// USE: Called as `mapping_uart_port` from handlers, repository impls, or sibling helpers.
pub(crate) fn mapping_uart_port(mappings: &serde_json::Value, mac: &str) -> Option<String> {
    mapping_entry_for_mac(mappings, mac)?
        .get("uart")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Remove mapping keys whose normalized form equals `normalized_key` (delete-device cleanup).
pub(crate) fn mappings_without_normalized_key(
    mappings: &serde_json::Value,
    normalized_key: &str,
) -> serde_json::Value {
    let Some(object) = mappings.as_object() else {
        return serde_json::json!({});
    };
    let filtered: serde_json::Map<String, serde_json::Value> = object
        .iter()
        .filter(|(key, _)| normalize_mapping_lookup_key(key) != normalized_key)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    serde_json::Value::Object(filtered)
}

/// PURPOSE: Serialize `test status str` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn test_status_str(status: Option<&test_execution::TestStatus>) -> Option<&'static str> {
    status.map(|s| match s {
        test_execution::TestStatus::NotExecuted => "not_executed",
        test_execution::TestStatus::InProgress => "in_progress",
        test_execution::TestStatus::Completed => "completed",
        test_execution::TestStatus::Queued => "queued",
        test_execution::TestStatus::Cancelled => "cancelled",
        test_execution::TestStatus::Failed => "failed",
    })
}

/// PURPOSE: Serialize `execution phase str` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn execution_phase_str(
    phase: Option<&test_execution::ExecutionPhase>,
) -> Option<&'static str> {
    phase.map(|p| match p {
        test_execution::ExecutionPhase::PrepareArtifacts => "PREPARE_ARTIFACTS",
        test_execution::ExecutionPhase::PrepareTestScripts => "PREPARE_TEST_SCRIPTS",
        test_execution::ExecutionPhase::QueueTestExecution => "QUEUE_TEST_EXECUTION",
        test_execution::ExecutionPhase::ProcessingQueued => "PROCESSING_QUEUED",
        test_execution::ExecutionPhase::AssigningDevice => "ASSIGNING_DEVICE",
        test_execution::ExecutionPhase::DeviceAssigned => "DEVICE_ASSIGNED",
        test_execution::ExecutionPhase::SendIplRequest => "SEND_IPL_REQUEST",
        test_execution::ExecutionPhase::WaitForIplConfirmation => "WAIT_FOR_IPL_CONFIRMATION",
        test_execution::ExecutionPhase::RequeueAfterIplConfirmation => {
            "REQUEUE_AFTER_IPL_CONFIRMATION"
        }
        test_execution::ExecutionPhase::SendFlashRequest => "SEND_FLASH_REQUEST",
        test_execution::ExecutionPhase::WaitForDeviceBoot => "WAIT_FOR_DEVICE_BOOT",
        test_execution::ExecutionPhase::StartTestExecution => "START_TEST_EXECUTION",
        test_execution::ExecutionPhase::SendFallbackFlashRequest => "SEND_FALLBACK_FLASH_REQUEST",
        test_execution::ExecutionPhase::WaitForFallbackCompletion => "WAIT_FOR_FALLBACK_COMPLETION",
        test_execution::ExecutionPhase::CompleteSuccess => "COMPLETE_SUCCESS",
        test_execution::ExecutionPhase::CompleteFailed => "COMPLETE_FAILED",
        test_execution::ExecutionPhase::CompleteCancelled => "COMPLETE_CANCELLED",
    })
}

/// PURPOSE: Serialize `test execution json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn test_execution_json(model: &test_execution::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "testId": model.test_id,
        "deviceFamily": model.device_family,
        "deviceType": model.device_type,
        "deviceId": model.device_id,
        "buildId": model.build_id,
        "buildVersion": model.build_version,
        "testPlanId": model.test_plan_id,
        "testSuits": model.test_suits,
        "selectionInput": model.selection_input,
        "logs": model.logs,
        "testCycleId": model.test_cycle_id,
        "status": test_status_str(model.status.as_ref()),
        "createdBy": model.created_by,
        "updatedBy": model.updated_by,
        "startedAt": model.started_at,
        "endedAt": model.ended_at,
        "testPlanName": model.test_plan_name,
        "name": model.name,
        "queueId": model.queue_id,
        "isAllSelected": model.is_all_selected,
        "executionPhase": execution_phase_str(model.execution_phase.as_ref()),
        "cancelRequested": model.cancel_requested,
        "cancelRequestedBy": model.cancel_requested_by,
        "cancelRequestedAt": model.cancel_requested_at,
        "cancelHandled": model.cancel_handled,
        "cancelPreserve": model.cancel_preserve,
        "cancelSystemReason": model.cancel_system_reason,
        "rtosLogPath": model.rtos_log_path,
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
    })
}

/// PURPOSE: Serialize `test case json` to the camelCase API JSON/string shape.
/// USE: JSON assembly matching legacy Sequelize `to_jsonb` / enum text.
pub(crate) fn test_case_json(model: &test_case::Model) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "testCaseId": model.test_case_id,
        "title": model.title,
        "executionId": model.execution_id,
        "result": model.result.as_ref().map(|r| match r {
            test_case::TestResult::Pass => "PASS",
            test_case::TestResult::Fail => "FAIL",
        }),
        "comment": model.comment,
        "cmd": model.cmd,
        "jiraDefect": model.jira_defect,
        "suiteId": model.suite_id,
        "scriptFile": model.script_file,
        "planId": model.plan_id,
        "priorityId": model.priority_id,
        "order": model.order,
        "suiteName": model.suite_name,
        "updatedAt": model.updated_at,
        "createdAt": model.created_at,
        "startedAt": model.started_at,
        "endedAt": model.ended_at,
        "outputFilePath": model.output_file_path,
        "dmesgFilePath": model.dmesg_file_path,
        "labels": model.labels,
        "preCondition": model.pre_condition,
    })
}

/// PURPOSE: Implement `duration_seconds` domain/persistence logic.
/// USE: Called as `duration_seconds` from handlers, repository impls, or sibling helpers.
pub(crate) fn duration_seconds(
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    ended_at: Option<chrono::DateTime<chrono::Utc>>,
) -> i64 {
    let now = chrono::Utc::now();
    let start = started_at.unwrap_or(now);
    let end = ended_at.unwrap_or(now);
    (end - start).num_seconds().max(0)
}

/// Matches `to_jsonb(te) || jsonb_build_object('Device', …, 'testCases', …, 'logs', …)`.
pub(crate) fn created_execution_json(
    execution: &test_execution::Model,
    cases: &[test_case::Model],
) -> serde_json::Value {
    let mut value = test_execution_json(execution);
    value["Device"] = serde_json::Value::Null;
    value["testCases"] = serde_json::Value::Array(cases.iter().map(test_case_json).collect());
    value["logs"] = serde_json::json!([]);
    value
}

/// Matches `to_jsonb(te) || jsonb_build_object('testCases', …)`.
pub(crate) fn execution_with_cases_json(
    execution: &test_execution::Model,
    cases: &[test_case::Model],
) -> serde_json::Value {
    let mut value = test_execution_json(execution);
    value["testCases"] = serde_json::Value::Array(cases.iter().map(test_case_json).collect());
    value
}
