use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, DatabaseBackend, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, Statement,
};

use crate::persistence::entity::device;

use super::super::{
    DeviceStateAnalytics, DeviceStateDetailedAnalytics, DeviceStateDetailedItem,
    error::DeviceRepositoryError,
};

#[derive(Debug, FromQueryResult)]
struct JsonRow {
    value: serde_json::Value,
}

pub(crate) async fn device_state_counts(
    orm: &DatabaseConnection,
) -> Result<DeviceStateAnalytics, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::state_usage",
        "device_state_counts",
    );
    let rows = device::Entity::find()
        .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
        .filter(device::Column::DeletedAt.is_null())
        .all(orm)
        .await?;
    let mut state_count: BTreeMap<String, i64> = BTreeMap::new();
    for row in &rows {
        let state = device_state_str(row.state.as_ref())
            .unwrap_or("unknown")
            .to_owned();
        *state_count.entry(state).or_insert(0) += 1;
    }
    Ok(DeviceStateAnalytics {
        total: rows.len() as i64,
        state_count,
    })
}

pub(crate) async fn device_state_details(
    orm: &DatabaseConnection,
) -> Result<DeviceStateDetailedAnalytics, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::state_usage",
        "device_state_details",
    );
    let rows = device::Entity::find()
        .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
        .filter(device::Column::DeletedAt.is_null())
        .order_by_asc(device::Column::DeviceFamily)
        .order_by_asc(device::Column::DeviceId)
        .all(orm)
        .await?;
    let mut analytics: BTreeMap<String, Vec<DeviceStateDetailedItem>> = BTreeMap::new();
    for row in rows {
        let family = row
            .device_family
            .clone()
            .unwrap_or_else(|| "Unknown".to_owned());
        analytics.entry(family).or_default().push(DeviceStateDetailedItem {
            device_id: row.device_id,
            device_type: row.device_type.unwrap_or_else(|| "Unknown".to_owned()),
            state: device_state_str(row.state.as_ref())
                .unwrap_or("unknown")
                .to_owned(),
        });
    }
    Ok(DeviceStateDetailedAnalytics(analytics))
}

pub(crate) async fn device_usage(
    orm: &DatabaseConnection,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    device_id: Option<&str>,
    device_family: Option<&str>,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::state_usage",
        "device_usage",
    );
    // Windowed duration aggregation over state-change history — keep as Statement.
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"WITH selected_devices AS (
                 SELECT "deviceId" FROM devices
                 WHERE status::text = 'approved' AND "deletedAt" IS NULL
                     AND "createdAt" <= $2
                     AND ($3::text IS NULL OR "deviceId" = $3)
                     AND ($4::text IS NULL OR "deviceFamily" = $4)
             ), device_created_times AS (
                 SELECT "deviceId", min("changedAt") FILTER (WHERE state::text = 'free') AS created_at
                 FROM device_state_change WHERE "deviceId" IN (SELECT "deviceId" FROM selected_devices)
                 GROUP BY "deviceId"
             ), state_logs AS (
                 SELECT dsc."deviceId", dsc.state::text AS state, dsc."changedAt" AS start_time,
                     lead(dsc."changedAt") OVER (PARTITION BY dsc."deviceId" ORDER BY dsc."changedAt") AS end_time
                 FROM device_state_change dsc WHERE dsc."changedAt" <= $2
                     AND dsc."deviceId" IN (SELECT "deviceId" FROM selected_devices)
             ), filled_logs AS (
                 SELECT sl."deviceId", sl.state,
                     greatest(sl.start_time, COALESCE(dct.created_at, $1), $1) AS start_time,
                     least(COALESCE(sl.end_time, $2, now()), $2, now()) AS end_time
                 FROM state_logs sl LEFT JOIN device_created_times dct ON sl."deviceId" = dct."deviceId"
             ), durations AS (
                 SELECT "deviceId", state, extract(epoch FROM (end_time - start_time)) AS duration_seconds
                 FROM filled_logs WHERE end_time > start_time
             ), fallback_states AS (
                 SELECT DISTINCT ON (dsc."deviceId") dsc."deviceId", dsc.state::text AS state,
                     greatest($1, COALESCE(dct.created_at, $1)) AS start_time, $2 AS end_time
                 FROM device_state_change dsc LEFT JOIN device_created_times dct
                     ON dct."deviceId" = dsc."deviceId"
                 WHERE dsc."changedAt" < $1
                     AND dsc."deviceId" IN (SELECT "deviceId" FROM selected_devices)
                 ORDER BY dsc."deviceId", dsc."changedAt" DESC
             ), combined AS (
                 SELECT * FROM durations
                 UNION ALL
                 SELECT "deviceId", state, extract(epoch FROM (end_time - start_time))
                 FROM fallback_states WHERE "deviceId" NOT IN (SELECT DISTINCT "deviceId" FROM durations)
             )
             SELECT jsonb_build_object(
                 'totalDevices', count(DISTINCT "deviceId"),
                 'totalSeconds', COALESCE(sum(duration_seconds), 0),
                 'states', jsonb_build_object(
                     'busy', COALESCE(sum(duration_seconds) FILTER (WHERE state = 'busy'), 0),
                     'not_reachable', COALESCE(sum(duration_seconds) FILTER (WHERE state = 'not_reachable'), 0),
                     'free', COALESCE(sum(duration_seconds) FILTER (WHERE state = 'free'), 0),
                     'faulty', COALESCE(sum(duration_seconds) FILTER (WHERE state = 'faulty'), 0))) AS value
             FROM combined"#,
        [
            start.into(),
            end.into(),
            device_id.map(str::to_owned).into(),
            device_family.map(str::to_owned).into(),
        ],
    ))
    .one(orm)
    .await?
    .map(|row| row.value)
    .unwrap_or_else(|| serde_json::json!({})))
}

fn device_state_str(state: Option<&device::DeviceState>) -> Option<&'static str> {
    state.map(|s| match s {
        device::DeviceState::Busy => "busy",
        device::DeviceState::NotReachable => "not_reachable",
        device::DeviceState::Free => "free",
        device::DeviceState::Faulty => "faulty",
        device::DeviceState::Unknown => "unknown",
    })
}
