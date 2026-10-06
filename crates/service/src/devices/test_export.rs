use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::{error::DeviceRepositoryError, repository_types::TestExportRow};

pub(crate) async fn rows(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<TestExportRow>, DeviceRepositoryError> {
    Ok(sqlx::query_as::<_, TestExportRow>(
        r#"SELECT COALESCE(te."testPlanName", '') AS test_plan_name,
             COALESCE(te.status::text, 'not_executed') AS status,
             d."deviceType" AS device_type, te."startedAt" AS started_at,
             te."endedAt" AS ended_at, te."testCycleId" AS test_cycle_id,
             count(tc.id) AS total,
             count(tc.id) FILTER (WHERE tc.result::text = 'PASS') AS passed,
             count(tc.id) FILTER (WHERE tc.result::text = 'FAIL') AS failed
           FROM test_executions te
           LEFT JOIN devices d ON d."deviceId" = te."deviceId"
           LEFT JOIN testcase tc ON tc."executionId" = te."testId"
           WHERE te."createdBy" = $1
           GROUP BY te.id, d."deviceType"
           ORDER BY te.id"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

pub(crate) fn format_rows(
    rows: &[TestExportRow],
    now: DateTime<Utc>,
) -> Result<String, DeviceRepositoryError> {
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
    value
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

fn csv_error(error: csv::Error) -> DeviceRepositoryError {
    DeviceRepositoryError::Internal(format!("Failed to generate CSV: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_test_execution_csv_calculations() {
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
