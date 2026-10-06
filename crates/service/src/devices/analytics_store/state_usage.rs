use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::super::{
    DeviceStateAnalytics, DeviceStateDetailedAnalytics, DeviceStateDetailedItem,
    error::DeviceRepositoryError,
};

#[derive(sqlx::FromRow)]
struct StateCountRow {
    state: String,
    count: i64,
}

#[derive(sqlx::FromRow)]
struct StateDetailRow {
    device_family: String,
    device_id: String,
    device_type: String,
    state: String,
}

pub(crate) async fn device_state_counts(
    pool: &PgPool,
) -> Result<DeviceStateAnalytics, DeviceRepositoryError> {
    let rows = sqlx::query_as::<_, StateCountRow>(
        r#"SELECT state::text AS state, count(*) AS count
           FROM devices WHERE status::text = 'approved' AND "deletedAt" IS NULL
           GROUP BY state ORDER BY state::text ASC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(DeviceStateAnalytics {
        total: rows.iter().map(|row| row.count).sum(),
        state_count: rows.into_iter().map(|row| (row.state, row.count)).collect(),
    })
}

pub(crate) async fn device_state_details(
    pool: &PgPool,
) -> Result<DeviceStateDetailedAnalytics, DeviceRepositoryError> {
    let rows = sqlx::query_as::<_, StateDetailRow>(
        r#"SELECT COALESCE("deviceFamily", 'Unknown') AS device_family,
             "deviceId" AS device_id, COALESCE("deviceType", 'Unknown') AS device_type,
             state::text AS state
           FROM devices WHERE status::text = 'approved' AND "deletedAt" IS NULL
           ORDER BY COALESCE("deviceFamily", 'Unknown') ASC, "deviceId" ASC"#,
    )
    .fetch_all(pool)
    .await?;
    let mut analytics: BTreeMap<String, Vec<DeviceStateDetailedItem>> = BTreeMap::new();
    for row in rows {
        analytics
            .entry(row.device_family)
            .or_default()
            .push(DeviceStateDetailedItem {
                device_id: row.device_id,
                device_type: row.device_type,
                state: row.state,
            });
    }
    Ok(DeviceStateDetailedAnalytics(analytics))
}

pub(crate) async fn device_usage(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    device_id: Option<&str>,
    device_family: Option<&str>,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
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
                     'faulty', COALESCE(sum(duration_seconds) FILTER (WHERE state = 'faulty'), 0)))
             FROM combined"#,
    )
    .bind(start)
    .bind(end)
    .bind(device_id)
    .bind(device_family)
    .fetch_one(pool)
    .await?)
}
