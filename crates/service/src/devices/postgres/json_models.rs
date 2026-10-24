//! Assemble API JSON from SeaORM entity models (replacing to_jsonb / jsonb_agg in SQL).

use crate::persistence::entity::{
    device, device_controller, device_interface, relay, relay_channel, test_case, test_execution,
};

pub(crate) fn device_status_str(status: Option<&device::DeviceStatus>) -> Option<&'static str> {
    status.map(|s| match s {
        device::DeviceStatus::Approved => "approved",
        device::DeviceStatus::Requested => "requested",
        device::DeviceStatus::Declined => "declined",
    })
}

pub(crate) fn device_state_str(state: Option<&device::DeviceState>) -> Option<&'static str> {
    state.map(|s| match s {
        device::DeviceState::Busy => "busy",
        device::DeviceState::NotReachable => "not_reachable",
        device::DeviceState::Free => "free",
        device::DeviceState::Faulty => "faulty",
        device::DeviceState::Unknown => "unknown",
    })
}

pub(crate) fn device_power_str(power: Option<&device::DevicePower>) -> Option<&'static str> {
    power.map(|p| match p {
        device::DevicePower::On => "on",
        device::DevicePower::Off => "off",
    })
}

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

pub(super) fn controller_json(model: &device_controller::Model) -> serde_json::Value {
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

pub(super) fn relay_json(model: &relay::Model) -> serde_json::Value {
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

pub(super) fn relay_channel_json(model: &relay_channel::Model) -> serde_json::Value {
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

pub(super) fn test_status_str(status: Option<&test_execution::TestStatus>) -> Option<&'static str> {
    status.map(|s| match s {
        test_execution::TestStatus::NotExecuted => "not_executed",
        test_execution::TestStatus::InProgress => "in_progress",
        test_execution::TestStatus::Completed => "completed",
        test_execution::TestStatus::Queued => "queued",
        test_execution::TestStatus::Cancelled => "cancelled",
        test_execution::TestStatus::Failed => "failed",
    })
}

pub(super) fn execution_phase_str(
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

pub(super) fn test_execution_json(model: &test_execution::Model) -> serde_json::Value {
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

pub(super) fn test_case_json(model: &test_case::Model) -> serde_json::Value {
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

pub(super) fn duration_seconds(
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    ended_at: Option<chrono::DateTime<chrono::Utc>>,
) -> i64 {
    let now = chrono::Utc::now();
    let start = started_at.unwrap_or(now);
    let end = ended_at.unwrap_or(now);
    (end - start).num_seconds().max(0)
}
