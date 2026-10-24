use chrono::Utc;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait,
    sea_query::{Expr, LockType},
};

use crate::persistence::entity::{
    device, device_action_queue, device_controller, relay, relay_channel,
};

use super::{
    error::DeviceRepositoryError,
    postgres::json_models::mapping_uart_port,
    repository_types::{DeviceFlashingResult, RtosCaptureTarget},
    validation::compare_versions,
};

#[derive(Debug, FromQueryResult)]
struct FlashingDevice {
    state: Option<device::DeviceState>,
    software_version: Option<String>,
    upgrading: Option<bool>,
    flashing: Option<bool>,
    device_family: Option<String>,
    mac_address: Option<String>,
    device_id: String,
}

pub(crate) async fn mark_device_flashing(
    orm: &DatabaseConnection,
    device_id: &str,
) -> Result<Option<DeviceFlashingResult>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::flashing_store", "mark_device_flashing");
    let transaction = orm.begin().await?;
    let device_row = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .lock(LockType::Update)
        .select_only()
        .column(device::Column::State)
        .column(device::Column::SoftwareVersion)
        .column(device::Column::Upgrading)
        .column(device::Column::Flashing)
        .column(device::Column::DeviceFamily)
        .column(device::Column::MacAddress)
        .column(device::Column::DeviceId)
        .into_model::<FlashingDevice>()
        .one(&transaction)
        .await?;
    let Some(device_row) = device_row else {
        transaction.rollback().await?;
        return Ok(None);
    };

    let state_text = match device_row.state {
        Some(device::DeviceState::Busy) => "busy",
        Some(device::DeviceState::Free) => "free",
        Some(device::DeviceState::Faulty) => "faulty",
        Some(device::DeviceState::NotReachable) => "not_reachable",
        Some(device::DeviceState::Unknown) | None => "unknown",
    };
    let upgrading_flag = device_row.upgrading.unwrap_or(false);
    let flashing_flag = device_row.flashing.unwrap_or(false);
    let mac_address = device_row
        .mac_address
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(device_row.device_id.as_str())
        .to_owned();

    let mut active_job_type = None;
    let (upgrading, flashing) = if state_text != "busy" {
        (upgrading_flag, flashing_flag)
    } else {
        #[derive(Debug, FromQueryResult)]
        struct ActiveJob {
            job_type: device_action_queue::QueueJobType,
            version: Option<String>,
        }
        let job = device_action_queue::Entity::find()
            .filter(device_action_queue::Column::TargetDeviceId.eq(device_id))
            .filter(device_action_queue::Column::Status.is_in([
                device_action_queue::QueueJobStatus::Waiting,
                device_action_queue::QueueJobStatus::Running,
            ]))
            .order_by_desc(device_action_queue::Column::CreatedAt)
            .limit(1)
            .select_only()
            .column_as(device_action_queue::Column::Type, "job_type")
            .column(device_action_queue::Column::Version)
            .into_model::<ActiveJob>()
            .one(&transaction)
            .await?;
        active_job_type = job.as_ref().map(|job| match job.job_type {
            device_action_queue::QueueJobType::Flash => "flash".to_owned(),
            device_action_queue::QueueJobType::Test => "test".to_owned(),
        });
        match job {
            Some(job) if matches!(job.job_type, device_action_queue::QueueJobType::Flash) => {
                match (
                    device_row.software_version.as_deref(),
                    job.version.as_deref(),
                ) {
                    (Some(current), Some(target)) => match compare_versions(current, target) {
                        1 => (true, false),
                        -1 => (false, true),
                        _ => (false, false),
                    },
                    _ => (false, false),
                }
            }
            _ => (false, true),
        }
    };

    let now = Utc::now();
    device::Entity::update_many()
        .col_expr(device::Column::Upgrading, Expr::value(Some(upgrading)))
        .col_expr(device::Column::Flashing, Expr::value(Some(flashing)))
        .col_expr(device::Column::UpdatedAt, Expr::value(now))
        .filter(device::Column::DeviceId.eq(device_id))
        .exec(&transaction)
        .await?;

    let updated_device = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .one(&transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::Internal("device disappeared after flashing update".to_owned())
        })?;
    // Flashing response historically returns a device-field subset (not full device_json).
    let updated = serde_json::json!({
        "id": updated_device.id,
        "deviceName": updated_device.device_name,
        "deviceFamily": updated_device.device_family,
        "deviceId": updated_device.device_id,
        "macAddress": updated_device.mac_address,
        "ipAddress": updated_device.ip_address,
        "deviceType": updated_device.device_type,
        "state": updated_device.state.as_ref().map(|state| match state {
            device::DeviceState::Busy => "busy",
            device::DeviceState::NotReachable => "not_reachable",
            device::DeviceState::Free => "free",
            device::DeviceState::Faulty => "faulty",
            device::DeviceState::Unknown => "unknown",
        }),
        "status": updated_device.status.as_ref().map(|status| match status {
            device::DeviceStatus::Approved => "approved",
            device::DeviceStatus::Requested => "requested",
            device::DeviceStatus::Declined => "declined",
        }),
        "upgrading": updated_device.upgrading,
        "flashing": updated_device.flashing,
        "softwareVersion": updated_device.software_version,
        "updatedAt": updated_device.updated_at,
        "createdAt": updated_device.created_at,
    });

    let family = device_row.device_family.as_deref().map(|value| {
        value
            .split_whitespace()
            .collect::<String>()
            .to_ascii_lowercase()
    });
    let rtos_target = if active_job_type.as_deref() == Some("test") {
        match family.as_deref() {
            Some("gen5") => {
                let controllers = device_controller::Entity::find()
                    .filter(
                        device_controller::Column::Status
                            .eq(device_controller::DeviceControllerStatus::Approved),
                    )
                    .filter(
                        device_controller::Column::State
                            .eq(device_controller::DeviceControllerState::Active),
                    )
                    .filter(device_controller::Column::DeletedAt.is_null())
                    .order_by_desc(device_controller::Column::UpdatedAt)
                    .all(&transaction)
                    .await?;
                controllers.into_iter().find_map(|controller| {
                    let uart_port = mapping_uart_port(&controller.mappings, &mac_address)?;
                    Some(RtosCaptureTarget {
                        controller_ip: controller.ip_address,
                        mac_address: mac_address.clone(),
                        generation: 5,
                        rtos_port: Some(next_uart_port(&uart_port)),
                        relay_serial: None,
                        relay_channel: None,
                    })
                })
            }
            Some("gen4") => {
                let row = relay_channel::Entity::find()
                    .filter(relay_channel::Column::DeviceId.eq(device_id))
                    .filter(relay_channel::Column::DeletedAt.is_null())
                    .find_also_related(relay::Entity)
                    .order_by_desc(relay_channel::Column::UpdatedAt)
                    .one(&transaction)
                    .await?;
                match row {
                    Some((channel, Some(relay_row))) if relay_row.deleted_at.is_none() => {
                        let controller = device_controller::Entity::find()
                            .filter(
                                device_controller::Column::DeviceControllerId
                                    .eq(&relay_row.device_controller_id),
                            )
                            .filter(device_controller::Column::DeletedAt.is_null())
                            .one(&transaction)
                            .await?;
                        controller.map(|controller| RtosCaptureTarget {
                            controller_ip: controller.ip_address,
                            mac_address: mac_address.clone(),
                            generation: 4,
                            rtos_port: None,
                            relay_serial: Some(relay_row.serial_number),
                            relay_channel: Some(channel.channel_number),
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    } else {
        None
    };
    transaction.commit().await?;
    Ok(Some(DeviceFlashingResult {
        device: updated,
        rtos_target,
    }))
}

fn next_uart_port(port: &str) -> String {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::flashing_store", "next_uart_port");
    let split = port
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_ascii_digit())
        .map_or(0, |(index, character)| index + character.len_utf8());
    let (prefix, suffix) = port.split_at(split);
    suffix.parse::<u32>().map_or_else(
        |_| port.to_owned(),
        |number| format!("{prefix}{}", number + 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtos_uart_is_adjacent_to_the_primary_uart() {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::flashing_store",
            "rtos_uart_is_adjacent_to_the_primary_uart",
        );
        assert_eq!(next_uart_port("/dev/ttyUSB1"), "/dev/ttyUSB2");
        assert_eq!(next_uart_port("ttyS"), "ttyS");
    }
}
