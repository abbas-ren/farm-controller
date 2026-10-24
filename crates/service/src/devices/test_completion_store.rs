use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    sea_query::Expr,
};

use crate::persistence::entity::{
    device, device_controller, relay, relay_channel, test_execution,
};

use super::{
    error::DeviceRepositoryError,
    postgres::json_models::{mapping_entry_for_mac, normalize_mapping_lookup_key},
    repository_types::TestCompletionTarget,
};

pub(crate) async fn test_completion_target(
    orm: &DatabaseConnection,
    test_id: &str,
    device_id: &str,
) -> Result<Option<TestCompletionTarget>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::test_completion_store",
        "test_completion_target",
    );
    let Some(test) = test_execution::Entity::find()
        .filter(test_execution::Column::TestId.eq(test_id))
        .one(orm)
        .await?
    else {
        return Ok(None);
    };

    let device_model = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .one(orm)
        .await?;

    let (gen4_controller_ip, relay_serial, relay_channel_number) =
        if let Some(ref device_model) = device_model {
            resolve_relay_mapping(orm, &device_model.device_id).await?
        } else {
            (None, None, None)
        };

    let (gen5_controller_ip, uart_port) = if let Some(ref device_model) = device_model {
        resolve_gen5_mapping(orm, device_model).await?
    } else {
        (None, None)
    };

    Ok(Some(TestCompletionTarget {
        status: super::postgres::json_models::test_status_str(test.status.as_ref())
            .unwrap_or("not_executed")
            .to_owned(),
        device_family: device_model.as_ref().and_then(|d| d.device_family.clone()),
        mac_address: device_model.as_ref().and_then(|d| d.mac_address.clone()),
        device_type: test.device_type,
        build_version: test.build_version,
        build_id: Some(test.build_id),
        created_by: test.created_by,
        gen5_controller_ip,
        uart_port,
        gen4_controller_ip,
        relay_serial,
        relay_channel: relay_channel_number,
    }))
}

async fn resolve_relay_mapping(
    orm: &DatabaseConnection,
    device_id: &str,
) -> Result<(Option<String>, Option<String>, Option<i32>), DeviceRepositoryError> {
    let channels = relay_channel::Entity::find()
        .filter(relay_channel::Column::DeviceId.eq(device_id))
        .filter(relay_channel::Column::DeletedAt.is_null())
        .order_by_desc(relay_channel::Column::UpdatedAt)
        .find_also_related(relay::Entity)
        .all(orm)
        .await?;

    for (channel, relay_model) in channels {
        let Some(relay_model) = relay_model.filter(|r| r.deleted_at.is_none()) else {
            continue;
        };
        let controller = device_controller::Entity::find()
            .filter(
                device_controller::Column::DeviceControllerId
                    .eq(&relay_model.device_controller_id),
            )
            .filter(device_controller::Column::DeletedAt.is_null())
            .one(orm)
            .await?;
        return Ok((
            controller.map(|c| c.ip_address),
            Some(relay_model.serial_number),
            Some(channel.channel_number),
        ));
    }
    Ok((None, None, None))
}

async fn resolve_gen5_mapping(
    orm: &DatabaseConnection,
    device_model: &device::Model,
) -> Result<(Option<String>, Option<String>), DeviceRepositoryError> {
    let Some(mac) = device_model.mac_address.as_deref() else {
        return Ok((None, None));
    };
    let mac_key = normalize_mapping_lookup_key(mac);
    if mac_key.is_empty() {
        return Ok((None, None));
    }

    let mut controllers = device_controller::Entity::find()
        .filter(device_controller::Column::State.eq(device_controller::DeviceControllerState::Active))
        .filter(
            device_controller::Column::Status.eq(device_controller::DeviceControllerStatus::Approved),
        )
        .filter(device_controller::Column::DeletedAt.is_null())
        .all(orm)
        .await?;

    controllers.retain(|c| mapping_entry_for_mac(&c.mappings, mac).is_some());
    controllers.sort_by(|a, b| {
        let a_preferred = device_model
            .controller_id
            .as_deref()
            .is_some_and(|id| id == a.device_controller_id);
        let b_preferred = device_model
            .controller_id
            .as_deref()
            .is_some_and(|id| id == b.device_controller_id);
        b_preferred
            .cmp(&a_preferred)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });

    let Some(controller) = controllers.into_iter().next() else {
        return Ok((None, None));
    };
    let uart = mapping_entry_for_mac(&controller.mappings, mac)
        .and_then(|entry| entry.get("uart"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    Ok((Some(controller.ip_address), uart))
}

pub(crate) async fn store_rtos_log_path(
    orm: &DatabaseConnection,
    test_id: &str,
    path: &str,
) -> Result<(), DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::test_completion_store",
        "store_rtos_log_path",
    );
    let now = chrono::Utc::now();
    test_execution::Entity::update_many()
        .col_expr(
            test_execution::Column::RtosLogPath,
            Expr::value(Some(path.to_owned())),
        )
        .col_expr(test_execution::Column::UpdatedAt, Expr::value(now))
        .filter(test_execution::Column::TestId.eq(test_id))
        .exec(orm)
        .await?;
    Ok(())
}
