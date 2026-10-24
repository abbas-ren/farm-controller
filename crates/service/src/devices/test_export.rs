use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

use crate::persistence::entity::{device, test_case, test_execution};

use super::{
    error::DeviceRepositoryError,
    postgres::json_models::test_status_str,
    repository_types::TestExportRow,
};

pub(crate) async fn rows(
    orm: &DatabaseConnection,
    user_id: &str,
) -> Result<Vec<TestExportRow>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::test_export", "rows");
    let executions = test_execution::Entity::find()
        .filter(test_execution::Column::CreatedBy.eq(user_id))
        .find_also_related(device::Entity)
        .order_by_asc(test_execution::Column::Id)
        .all(orm)
        .await?;

    let mut rows = Vec::with_capacity(executions.len());
    for (execution, device_model) in executions {
        let cases = test_case::Entity::find()
            .filter(test_case::Column::ExecutionId.eq(&execution.test_id))
            .all(orm)
            .await?;
        let total = cases.len() as i64;
        let passed = cases
            .iter()
            .filter(|c| matches!(c.result, Some(test_case::TestResult::Pass)))
            .count() as i64;
        let failed = cases
            .iter()
            .filter(|c| matches!(c.result, Some(test_case::TestResult::Fail)))
            .count() as i64;
        rows.push(TestExportRow {
            test_plan_name: execution.test_plan_name.clone().unwrap_or_default(),
            status: test_status_str(execution.status.as_ref())
                .unwrap_or("not_executed")
                .to_owned(),
            device_type: device_model.and_then(|d| d.device_type),
            started_at: execution.started_at,
            ended_at: execution.ended_at,
            test_cycle_id: execution.test_cycle_id,
            total,
            passed,
            failed,
        });
    }
    Ok(rows)
}

pub(crate) fn format_rows(
    rows: &[TestExportRow],
    now: DateTime<Utc>,
) -> Result<String, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::test_export", "format_rows");
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .write_record([
            "S.No",
            "Test Name",
            "Status",
            "Device",
            "Duration",
            "Total",
            "Passed",
            "Failed",
            "Qmetry Test Cycle Id",
            "Execution percentage",
            "Started At",
            "Ended At",
        ])
        .map_err(csv_error)?;
    for (index, row) in rows.iter().enumerate() {
        let status = if row.status == "completed" && row.total != row.passed + row.failed {
            "failed"
        } else {
            &row.status
        };
        let duration = row
            .started_at
            .map(|started| (row.ended_at.unwrap_or(now) - started).num_seconds().max(0))
            .unwrap_or(0);
        let percentage = if row.total == 0 {
            "false".to_owned()
        } else {
            ((row.passed + row.failed) * 100 / row.total).to_string()
        };
        writer
            .write_record([
                (index + 1).to_string(),
                row.test_plan_name.clone(),
                sentence_case(status),
                row.device_type.clone().unwrap_or_default(),
                format_duration(duration),
                row.total.to_string(),
                row.passed.to_string(),
                row.failed.to_string(),
                row.test_cycle_id.clone().unwrap_or_default(),
                percentage,
                iso(row.started_at),
                iso(row.ended_at),
            ])
            .map_err(csv_error)?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|error| csv_error(error.into_error().into()))?;
    String::from_utf8(bytes).map_err(|error| DeviceRepositoryError::Internal(error.to_string()))
}

fn sentence_case(value: &str) -> String {
    let _api_trace = crate::observability::ApiTrace::enter("devices::test_export", "sentence_case");
    value
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_duration(mut seconds: i64) -> String {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::test_export", "format_duration");
    let units = [
        (2_592_000, "Month"),
        (604_800, "Week"),
        (86_400, "Day"),
        (3_600, "Hr"),
        (60, "Min"),
        (1, "Sec"),
    ];
    let mut parts = Vec::new();
    for (size, label) in units {
        let amount = seconds / size;
        if amount > 0 {
            parts.push(format!("{amount} {label}"));
            seconds %= size;
        }
    }
    if parts.is_empty() {
        "0 Sec".to_owned()
    } else {
        parts.join(" ")
    }
}

fn iso(value: Option<DateTime<Utc>>) -> String {
    let _api_trace = crate::observability::ApiTrace::enter("devices::test_export", "iso");
    value
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

fn csv_error(error: csv::Error) -> DeviceRepositoryError {
    let _api_trace = crate::observability::ApiTrace::enter("devices::test_export", "csv_error");
    DeviceRepositoryError::Internal(format!("Failed to generate CSV: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_test_execution_csv_calculations() {
        let _api_trace = crate::observability::ApiTrace::enter(
            "devices::test_export",
            "formats_test_execution_csv_calculations",
        );
        let started = "2026-10-02T00:00:00Z".parse().unwrap();
        let ended = "2026-10-02T01:01:01Z".parse().unwrap();
        let csv = format_rows(
            &[TestExportRow {
                test_plan_name: "Smoke".to_owned(),
                status: "completed".to_owned(),
                device_type: Some("x5h".to_owned()),
                started_at: Some(started),
                ended_at: Some(ended),
                test_cycle_id: Some("C-1".to_owned()),
                total: 3,
                passed: 1,
                failed: 1,
            }],
            ended,
        )
        .unwrap();
        assert!(csv.contains("Smoke,Failed,x5h,1 Hr 1 Min 1 Sec,3,1,1,C-1,66"));
    }
}
