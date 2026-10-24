use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};

use crate::persistence::entity::{device, device_state_change};

use super::super::{
    DeviceStateAnalytics, DeviceStateDetailedAnalytics, DeviceStateDetailedItem,
    error::DeviceRepositoryError,
};

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
    let mut devices_query = device::Entity::find()
        .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
        .filter(device::Column::DeletedAt.is_null())
        .filter(device::Column::CreatedAt.lte(end));
    if let Some(device_id) = device_id {
        devices_query = devices_query.filter(device::Column::DeviceId.eq(device_id));
    }
    if let Some(device_family) = device_family {
        devices_query = devices_query.filter(device::Column::DeviceFamily.eq(device_family));
    }
    let selected: Vec<String> = devices_query
        .all(orm)
        .await?
        .into_iter()
        .map(|row| row.device_id)
        .collect();
    if selected.is_empty() {
        return Ok(serde_json::json!({
            "totalDevices": 0,
            "totalSeconds": 0.0,
            "states": {
                "busy": 0.0,
                "not_reachable": 0.0,
                "free": 0.0,
                "faulty": 0.0,
            }
        }));
    }

    let changes = device_state_change::Entity::find()
        .filter(device_state_change::Column::DeviceId.is_in(selected.clone()))
        .order_by_asc(device_state_change::Column::DeviceId)
        .order_by_asc(device_state_change::Column::ChangedAt)
        .all(orm)
        .await?;

    let mut created_at: HashMap<String, DateTime<Utc>> = HashMap::new();
    let mut by_device: HashMap<String, Vec<&device_state_change::Model>> = HashMap::new();
    for change in &changes {
        by_device
            .entry(change.device_id.clone())
            .or_default()
            .push(change);
        if matches!(
            change.state,
            device_state_change::DeviceStateChangeState::Free
        ) {
            created_at
                .entry(change.device_id.clone())
                .and_modify(|current| {
                    if change.changed_at < *current {
                        *current = change.changed_at;
                    }
                })
                .or_insert(change.changed_at);
        }
    }

    let now = Utc::now();
    let mut durations: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for (device_id, logs) in &by_device {
        let device_created = created_at.get(device_id).copied();
        for (index, log) in logs.iter().enumerate() {
            if log.changed_at > end {
                continue;
            }
            let start_time = log
                .changed_at
                .max(device_created.unwrap_or(start))
                .max(start);
            let end_time = logs
                .get(index + 1)
                .map(|next| next.changed_at)
                .unwrap_or(end.min(now))
                .min(end)
                .min(now);
            if end_time > start_time {
                let seconds = (end_time - start_time).num_milliseconds() as f64 / 1000.0;
                durations
                    .entry(device_id.clone())
                    .or_default()
                    .push((change_state_str(&log.state).to_owned(), seconds));
            }
        }
    }

    let devices_with_duration: HashSet<String> = durations.keys().cloned().collect();
    for (device_id, logs) in &by_device {
        if devices_with_duration.contains(device_id) {
            continue;
        }
        let fallback = logs
            .iter()
            .rev()
            .find(|log| log.changed_at < start);
        let Some(fallback) = fallback else {
            continue;
        };
        let device_created = created_at.get(device_id).copied();
        let start_time = start.max(device_created.unwrap_or(start));
        if end > start_time {
            let seconds = (end - start_time).num_milliseconds() as f64 / 1000.0;
            durations
                .entry(device_id.clone())
                .or_default()
                .push((change_state_str(&fallback.state).to_owned(), seconds));
        }
    }

    let mut total_seconds = 0.0_f64;
    let mut states = BTreeMap::from([
        ("busy".to_owned(), 0.0_f64),
        ("not_reachable".to_owned(), 0.0_f64),
        ("free".to_owned(), 0.0_f64),
        ("faulty".to_owned(), 0.0_f64),
    ]);
    let mut devices = HashSet::new();
    for (device_id, entries) in durations {
        devices.insert(device_id);
        for (state, seconds) in entries {
            total_seconds += seconds;
            *states.entry(state).or_insert(0.0) += seconds;
        }
    }

    Ok(serde_json::json!({
        "totalDevices": devices.len() as i64,
        "totalSeconds": total_seconds,
        "states": {
            "busy": states.get("busy").copied().unwrap_or(0.0),
            "not_reachable": states.get("not_reachable").copied().unwrap_or(0.0),
            "free": states.get("free").copied().unwrap_or(0.0),
            "faulty": states.get("faulty").copied().unwrap_or(0.0),
        }
    }))
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

fn change_state_str(state: &device_state_change::DeviceStateChangeState) -> &'static str {
    match state {
        device_state_change::DeviceStateChangeState::Busy => "busy",
        device_state_change::DeviceStateChangeState::NotReachable => "not_reachable",
        device_state_change::DeviceStateChangeState::Free => "free",
        device_state_change::DeviceStateChangeState::Faulty => "faulty",
        device_state_change::DeviceStateChangeState::Unknown => "unknown",
    }
}
