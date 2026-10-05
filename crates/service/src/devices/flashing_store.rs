use sqlx::PgPool;

use super::{
    error::DeviceRepositoryError,
    repository_types::{DeviceFlashingResult, RtosCaptureTarget},
    validation::compare_versions,
};

#[derive(sqlx::FromRow)]
struct FlashingDevice {
    state: String,
    software_version: Option<String>,
    upgrading: bool,
    flashing: bool,
    device_family: Option<String>,
    mac_address: String,
}

pub(crate) async fn mark_device_flashing(
    pool: &PgPool,
    device_id: &str,
) -> Result<Option<DeviceFlashingResult>, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let device = sqlx::query_as::<_, FlashingDevice>(
        r#"SELECT state::text AS state, "softwareVersion" AS software_version,
                  upgrading, flashing, "deviceFamily" AS device_family,
                  "macAddress" AS mac_address
           FROM devices WHERE "deviceId" = $1 AND "deletedAt" IS NULL FOR UPDATE"#,
    )
    .bind(device_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(device) = device else {
        transaction.rollback().await?;
        return Ok(None);
    };

    let mut active_job_type = None;
    let (upgrading, flashing) = if device.state != "busy" {
        (device.upgrading, device.flashing)
    } else {
        let job = sqlx::query_as::<_, (String, Option<String>)>(
            r#"SELECT type::text, version FROM device_action_queue
               WHERE "targetDeviceId" = $1 AND status::text IN ('waiting', 'running')
               ORDER BY "createdAt" DESC LIMIT 1"#,
        )
        .bind(device_id)
        .fetch_optional(&mut *transaction)
        .await?;
        active_job_type = job.as_ref().map(|(job_type, _)| job_type.clone());
        match job {
            Some((job_type, target_version)) if job_type == "flash" => {
                match (
                    device.software_version.as_deref(),
                    target_version.as_deref(),
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
    let updated = sqlx::query_scalar::<_, serde_json::Value>(
        r#"UPDATE devices SET upgrading = $2, flashing = $3, "updatedAt" = now()
           WHERE "deviceId" = $1 RETURNING to_jsonb(devices)"#,
    )
    .bind(device_id)
    .bind(upgrading)
    .bind(flashing)
    .fetch_one(&mut *transaction)
    .await?;
    let family = device.device_family.as_deref().map(|value| {
        value
            .split_whitespace()
            .collect::<String>()
            .to_ascii_lowercase()
    });
    let rtos_target = if active_job_type.as_deref() == Some("test") {
        match family.as_deref() {
            Some("gen5") => sqlx::query_as::<_, (String, String)>(
                r#"SELECT controller."ipAddress",
                          entry.value ->> 'uart' AS uart_port
                   FROM device_controllers controller
                   CROSS JOIN LATERAL jsonb_each(COALESCE(controller.mappings, '{}'::jsonb)) entry
                   WHERE controller.status::text = 'approved'
                     AND controller.state::text = 'active'
                     AND controller."deletedAt" IS NULL
                     AND regexp_replace(lower(entry.key), '[:-]', '', 'g') =
                         regexp_replace(lower($1), '[:-]', '', 'g')
                     AND COALESCE(entry.value ->> 'uart', '') <> ''
                   ORDER BY controller."updatedAt" DESC LIMIT 1"#,
            )
            .bind(&device.mac_address)
            .fetch_optional(&mut *transaction)
            .await?
            .map(|(controller_ip, uart_port)| RtosCaptureTarget {
                controller_ip,
                mac_address: device.mac_address.clone(),
                generation: 5,
                rtos_port: Some(next_uart_port(&uart_port)),
                relay_serial: None,
                relay_channel: None,
            }),
            Some("gen4") => sqlx::query_as::<_, (String, String, i32)>(
                r#"SELECT controller."ipAddress", relay."serialNumber",
                          channel."channelNumber"
                   FROM relay_channels channel
                   JOIN relays relay ON relay.id = channel."relayId"
                     AND relay."deletedAt" IS NULL
                   JOIN device_controllers controller
                     ON controller."deviceControllerId" = relay."deviceControllerId"
                    AND controller."deletedAt" IS NULL
                   WHERE channel."deviceId" = $1 AND channel."deletedAt" IS NULL
                   ORDER BY channel."updatedAt" DESC LIMIT 1"#,
            )
            .bind(device_id)
            .fetch_optional(&mut *transaction)
            .await?
            .map(
                |(controller_ip, relay_serial, relay_channel)| RtosCaptureTarget {
                    controller_ip,
                    mac_address: device.mac_address.clone(),
                    generation: 4,
                    rtos_port: None,
                    relay_serial: Some(relay_serial),
                    relay_channel: Some(relay_channel),
                },
            ),
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
        assert_eq!(next_uart_port("/dev/ttyUSB1"), "/dev/ttyUSB2");
        assert_eq!(next_uart_port("ttyS"), "ttyS");
    }
}
