use chrono::{DateTime, Datelike, Days, Months, NaiveDate, Utc};
use sea_orm::{DatabaseBackend, DatabaseConnection, FromQueryResult, Statement};

use super::super::error::DeviceRepositoryError;

#[derive(Debug, FromQueryResult)]
struct TestCaseAnalyticsRow {
    created_at: DateTime<Utc>,
    result: String,
}

#[derive(Debug, FromQueryResult)]
struct JsonRow {
    value: serde_json::Value,
}

pub(crate) async fn recent_executions(
    orm: &DatabaseConnection,
    user_id: &str,
    count: i64,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "recent_executions",
    );
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT jsonb_build_object(
                 'testId', te."testId", 'buildVersion', te."buildVersion",
                 'status', te.status, 'testPlanName', te."testPlanName",
                 'createdAt', te."createdAt", 'deviceType', te."deviceType",
                 'startedAt', te."startedAt", 'endedAt', te."endedAt",
                 'totalTestCases', (SELECT count(*) FROM testcase tc
                     WHERE tc."executionId" = te."testId")) AS value
             FROM test_executions te WHERE te."createdBy" = $1
             ORDER BY te."createdAt" DESC LIMIT $2"#,
        [user_id.into(), count.into()],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect())
}

pub(crate) async fn execution_by_id(
    orm: &DatabaseConnection,
    test_id: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "execution_by_id",
    );
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT jsonb_build_object(
                 'testId', te."testId", 'buildVersion', te."buildVersion",
                 'status', te.status, 'testPlanName', te."testPlanName",
                 'createdAt', te."createdAt",
                 'totalTestCases', (SELECT count(*) FROM testcase tc
                     WHERE tc."executionId" = te."testId")) AS value
             FROM test_executions te WHERE te."testId" = $1
             ORDER BY te."createdAt" DESC LIMIT 1"#,
        [test_id.into()],
    ))
    .one(orm)
    .await?
    .map(|row| row.value))
}

pub(crate) async fn execution_daily(
    orm: &DatabaseConnection,
    user_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "execution_daily",
    );
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"WITH days AS (
                 SELECT generate_series($2::date, $3::date, interval '1 day')::date AS day
             )
             SELECT jsonb_build_object(
                 'date', to_char(days.day, 'YYYY-MM-DD'),
                 'totalExecutions', count(te."testId"),
                 'passedTestCases', COALESCE(sum((SELECT count(*) FROM testcase tc
                     WHERE tc."executionId" = te."testId" AND tc.result::text = 'PASS')), 0),
                 'failedTestCases', COALESCE(sum((SELECT count(*) FROM testcase tc
                     WHERE tc."executionId" = te."testId" AND tc.result::text = 'FAIL')), 0),
                 'totalDurationSeconds', COALESCE(sum(GREATEST(0, floor(extract(epoch FROM (
                     COALESCE(te."endedAt", now()) - COALESCE(te."startedAt", te."createdAt")
                 )))::bigint)), 0)) AS value
             FROM days LEFT JOIN test_executions te
                 ON te."createdBy" = $1
                 AND te.status::text IN ('completed', 'in_progress')
                 AND te."createdAt" >= days.day
                 AND te."createdAt" < days.day + interval '1 day'
             GROUP BY days.day ORDER BY days.day ASC"#,
        [user_id.into(), from.into(), to.into()],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect())
}

pub(crate) async fn test_execution_analytics(
    orm: &DatabaseConnection,
    user_id: &str,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "test_execution_analytics",
    );
    let rows = TestCaseAnalyticsRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT te."createdAt" AS created_at, tc.result::text AS result
           FROM test_executions te JOIN testcase tc ON tc."executionId" = te."testId"
           WHERE te."createdBy" = $1
                         AND tc.result IS NOT NULL
             AND te.status::text NOT IN ('cancelled', 'queued', 'not_executed', 'failed')"#,
        [user_id.into()],
    ))
    .all(orm)
    .await?;
    let today = Utc::now().date_naive();
    let current_week = today
        .checked_sub_days(Days::new(u64::from(today.weekday().num_days_from_sunday())))
        .unwrap_or(today);
    let current_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let periodic_start = current_month
        .checked_sub_months(Months::new(4))
        .unwrap_or(current_month);
    let all_time = aggregate_test_cases(rows.iter());
    let mut weekly = Vec::with_capacity(6);
    let mut monthly = Vec::with_capacity(6);
    for offset in (0..6).rev() {
        let week_start = current_week
            .checked_sub_days(Days::new(offset * 7))
            .unwrap_or(current_week);
        let week_end = week_start
            .checked_add_days(Days::new(6))
            .unwrap_or(week_start);
        let mut counts = aggregate_test_cases(rows.iter().filter(|row| {
            let day = row.created_at.date_naive();
            day >= periodic_start && day >= week_start && day <= week_end
        }));
        counts["week"] = serde_json::json!(format!(
            "{} - {}",
            week_start.format("%d %b"),
            week_end.format("%d %b")
        ));
        weekly.push(counts);

        let month_start = current_month
            .checked_sub_months(Months::new(offset as u32))
            .unwrap_or(current_month);
        let month_end = month_start
            .checked_add_months(Months::new(1))
            .and_then(|next| next.checked_sub_days(Days::new(1)))
            .unwrap_or(month_start);
        let mut counts = aggregate_test_cases(rows.iter().filter(|row| {
            let day = row.created_at.date_naive();
            day >= periodic_start && day >= month_start && day <= month_end
        }));
        counts["month"] = serde_json::json!(month_start.format("%b %Y").to_string());
        monthly.push(counts);
    }
    Ok(serde_json::json!({
        "allTime": all_time,
        "weekly": weekly,
        "monthly": monthly,
    }))
}

pub(crate) async fn in_progress_test_analytics(
    orm: &DatabaseConnection,
    user_id: &str,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "in_progress_test_analytics",
    );
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT jsonb_build_object(
                 'id', te.id, 'testPlanName', te."testPlanName", 'createdAt', te."createdAt",
                 'startedAt', te."startedAt", 'endedAt', te."endedAt", 'status', te.status,
                 'total', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId"),
                 'executed', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId"
                   AND tc.result::text IN ('PASS', 'FAIL'))) AS value
           FROM test_executions te WHERE te."createdBy" = $1 AND te.status::text = 'in_progress'
           ORDER BY te."createdAt" DESC"#,
        [user_id.into()],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect())
}

pub(crate) async fn test_plan_summary(
    orm: &DatabaseConnection,
    user_id: &str,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "test_plan_summary",
    );
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT jsonb_build_object(
                 'total', count(*),
                 'completed', count(*) FILTER (WHERE te.status::text = 'completed'
                   AND NOT EXISTS(SELECT 1 FROM testcase tc WHERE tc."executionId" = te."testId"
                     AND tc.result::text = 'FAIL')),
                 'failed', count(*) FILTER (WHERE te.status::text = 'failed' OR
                   (te.status::text = 'completed' AND EXISTS(SELECT 1 FROM testcase tc
                     WHERE tc."executionId" = te."testId" AND tc.result::text = 'FAIL'))),
                 'cancelled', count(*) FILTER (WHERE te.status::text = 'cancelled'),
                 'inProgress', count(*) FILTER (WHERE te.status::text NOT IN
                   ('completed', 'failed', 'cancelled'))) AS value
           FROM test_executions te WHERE te."createdBy" = $1"#,
        [user_id.into()],
    ))
    .one(orm)
    .await?
    .map(|row| row.value)
    .unwrap_or_else(|| serde_json::json!({})))
}

pub(crate) async fn daily_test_summary(
    orm: &DatabaseConnection,
    user_id: &str,
    days: i64,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "daily_test_summary",
    );
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"WITH days AS (
                 SELECT generate_series(current_date - ($2::int - 1), current_date,
                   interval '1 day')::date AS day
               )
               SELECT jsonb_build_object(
                 'date', to_char(days.day, 'YYYY-MM-DD'),
                 'totalExecutionSeconds', COALESCE(sum(GREATEST(0, floor(extract(epoch FROM (
                   COALESCE(te."endedAt", now()) - COALESCE(te."startedAt", te."createdAt")
                 )))::bigint)), 0),
                 'passed', COALESCE(sum((SELECT count(*) FROM testcase tc
                   WHERE tc."executionId" = te."testId" AND tc.result::text = 'PASS')), 0),
                 'failed', COALESCE(sum((SELECT count(*) FROM testcase tc
                   WHERE tc."executionId" = te."testId" AND tc.result::text = 'FAIL')), 0)) AS value
               FROM days LEFT JOIN test_executions te ON te."createdBy" = $1
                 AND te.status::text <> 'cancelled'
                 AND te."createdAt" >= days.day AND te."createdAt" < days.day + interval '1 day'
               GROUP BY days.day ORDER BY days.day ASC"#,
        [user_id.into(), days.into()],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect())
}

fn aggregate_test_cases<'a>(
    rows: impl Iterator<Item = &'a TestCaseAnalyticsRow>,
) -> serde_json::Value {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::analytics_store::executions",
        "aggregate_test_cases",
    );
    let mut total = 0_i64;
    let mut passed = 0_i64;
    let mut failed = 0_i64;
    let mut in_progress = 0_i64;
    for row in rows {
        total += 1;
        match row.result.as_str() {
            "PASS" => passed += 1,
            "FAIL" => failed += 1,
            _ => in_progress += 1,
        }
    }
    serde_json::json!({
        "total": total,
        "passed": passed,
        "failed": failed,
        "inProgress": in_progress,
    })
}
