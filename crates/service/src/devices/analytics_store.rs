use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, Utc};
use sqlx::PgPool;

use super::{
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

#[derive(sqlx::FromRow)]
struct TestCaseAnalyticsRow {
    created_at: DateTime<Utc>,
    result: String,
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

pub(crate) async fn recent_executions(
    pool: &PgPool,
    user_id: &str,
    count: i64,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
                         'testId', te."testId", 'buildVersion', te."buildVersion",
                         'status', te.status, 'testPlanName', te."testPlanName",
                         'createdAt', te."createdAt", 'deviceType', te."deviceType",
                         'startedAt', te."startedAt", 'endedAt', te."endedAt",
                         'totalTestCases', (SELECT count(*) FROM testcase tc
                             WHERE tc."executionId" = te."testId"))
                     FROM test_executions te WHERE te."createdBy" = $1
                     ORDER BY te."createdAt" DESC LIMIT $2"#,
    )
    .bind(user_id)
    .bind(count)
    .fetch_all(pool)
    .await?)
}

pub(crate) async fn execution_by_id(
    pool: &PgPool,
    test_id: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
                         'testId', te."testId", 'buildVersion', te."buildVersion",
                         'status', te.status, 'testPlanName', te."testPlanName",
                         'createdAt', te."createdAt",
                         'totalTestCases', (SELECT count(*) FROM testcase tc
                             WHERE tc."executionId" = te."testId"))
                     FROM test_executions te WHERE te."testId" = $1
                     ORDER BY te."createdAt" DESC LIMIT 1"#,
    )
    .bind(test_id)
    .fetch_optional(pool)
    .await?)
}

pub(crate) async fn execution_daily(
    pool: &PgPool,
    user_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
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
                         )))::bigint)), 0))
                     FROM days LEFT JOIN test_executions te
                         ON te."createdBy" = $1
                         AND te.status::text IN ('completed', 'in_progress')
                         AND te."createdAt" >= days.day
                         AND te."createdAt" < days.day + interval '1 day'
                     GROUP BY days.day ORDER BY days.day ASC"#,
        )
        .bind(user_id)
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await?)
}

pub(crate) async fn build_comparisons(
    pool: &PgPool,
    user_id: &str,
    count: i64,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
                r#"WITH builds AS (
                         SELECT te."buildId" AS build_id, max(te."createdAt") AS last_used
                         FROM test_executions te JOIN releases r ON r.id::text = te."buildId"
                         WHERE te."createdBy" = $1 AND te.status::text = 'completed'
                             AND te."buildId" IS NOT NULL
                         GROUP BY te."buildId"
                     ), metrics AS (
                         SELECT b.build_id, b.last_used,
                             (SELECT count(*) FROM test_executions te
                                 WHERE te."buildId" = b.build_id AND te.status::text = 'completed') AS executions,
                             (SELECT COALESCE(sum(GREATEST(0, extract(epoch FROM (
                                     COALESCE(te."endedAt", now()) - COALESCE(te."startedAt", te."createdAt")
                                 )) * 1000)), 0) FROM test_executions te
                                 WHERE te."buildId" = b.build_id AND te.status::text = 'completed') AS duration_ms,
                             (SELECT count(*) FROM testcase tc JOIN test_executions te
                                 ON te."testId" = tc."executionId"
                                 WHERE te."buildId" = b.build_id AND te.status::text = 'completed') AS total_cases,
                             (SELECT count(*) FROM testcase tc JOIN test_executions te
                                 ON te."testId" = tc."executionId"
                                 WHERE te."buildId" = b.build_id AND te.status::text = 'completed'
                                     AND tc.result::text = 'PASS') AS passed_cases
                         FROM builds b
                     )
                     SELECT jsonb_build_object(
                         'buildId', m.build_id, 'buildVersion', r.version,
                         'deviceType', r."deviceType",
                         'averageDuration', CASE WHEN m.executions > 0
                             THEN round((m.duration_ms / m.executions)::numeric, 2) ELSE 0 END,
                         'totalTestCases', m.total_cases,
                         'passedPercentage', CASE WHEN m.total_cases > 0
                             THEN round((m.passed_cases::numeric / m.total_cases) * 100, 2) ELSE 0 END)
                     FROM metrics m JOIN releases r ON r.id::text = m.build_id
                     ORDER BY CASE WHEN m.total_cases > 0
                         THEN m.passed_cases::numeric / m.total_cases ELSE 0 END DESC,
                         m.last_used DESC LIMIT $2"#,
        )
        .bind(user_id)
        .bind(count)
        .fetch_all(pool)
        .await?)
}

pub(crate) async fn build_comparison_by_id(
    pool: &PgPool,
    user_id: &str,
    build_id: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
                r#"SELECT jsonb_build_object(
                         'buildId', $2::text,
                         'buildVersion', COALESCE((SELECT version FROM releases WHERE id::text = $2),
                             (SELECT "buildVersion" FROM test_executions WHERE "buildId" = $2
                                 AND status::text = 'completed' ORDER BY "createdAt" DESC LIMIT 1)),
                         'deviceType', (SELECT "deviceType" FROM releases WHERE id::text = $2),
                         'averageDuration', COALESCE((SELECT round(avg(GREATEST(0,
                             extract(epoch FROM (COALESCE("endedAt", now()) -
                                 COALESCE("startedAt", "createdAt"))) * 1000))::numeric, 2)
                             FROM test_executions WHERE "buildId" = $2 AND status::text = 'completed'), 0),
                         'totalTestCases', (SELECT count(*) FROM testcase tc JOIN test_executions te
                             ON te."testId" = tc."executionId"
                             WHERE te."buildId" = $2 AND te.status::text = 'completed'),
                         'passedPercentage', COALESCE((SELECT round(
                             count(*) FILTER (WHERE tc.result::text = 'PASS')::numeric /
                                 NULLIF(count(*), 0) * 100, 2)
                             FROM testcase tc JOIN test_executions te ON te."testId" = tc."executionId"
                             WHERE te."buildId" = $2 AND te.status::text = 'completed'), 0))
                     WHERE EXISTS(SELECT 1 FROM test_executions
                         WHERE "createdBy" = $1 AND "buildId" = $2 AND status::text = 'completed')"#,
        )
        .bind(user_id)
        .bind(build_id)
        .fetch_optional(pool)
        .await?)
}

pub(crate) async fn build_performance(
    pool: &PgPool,
    count: i64,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
                r#"WITH builds AS (
                         SELECT te."buildId" AS build_id, max(te."createdAt") AS last_used
                         FROM test_executions te JOIN releases r ON r.id::text = te."buildId"
                         WHERE te."buildId" IS NOT NULL GROUP BY te."buildId"
                         ORDER BY max(te."createdAt") DESC LIMIT $1
                     )
                     SELECT jsonb_build_object(
                         'buildId', b.build_id, 'buildVersion', COALESCE(r.version,
                             (SELECT "buildVersion" FROM test_executions WHERE "buildId" = b.build_id
                                 ORDER BY "createdAt" DESC LIMIT 1)),
                         'flagged', COALESCE(r."isFaulty", false), 'deviceType', r."deviceType",
                         'status', COALESCE(r.status::text, (SELECT status::text FROM test_executions
                             WHERE "buildId" = b.build_id ORDER BY "createdAt" DESC LIMIT 1)),
                         'lastTestExecutionId', (SELECT "testId" FROM test_executions
                             WHERE "buildId" = b.build_id ORDER BY "createdAt" DESC LIMIT 1),
                         'passedTestCaseCount', (SELECT count(*) FROM testcase tc JOIN test_executions te
                             ON te."testId" = tc."executionId" WHERE te."buildId" = b.build_id
                                 AND tc.result::text = 'PASS'),
                         'failedTestCaseCount', (SELECT count(*) FROM testcase tc JOIN test_executions te
                             ON te."testId" = tc."executionId" WHERE te."buildId" = b.build_id
                                 AND tc.result::text = 'FAIL'),
                         'passedTestCasePercentage', COALESCE((SELECT round(
                             count(*) FILTER (WHERE tc.result::text = 'PASS')::numeric /
                                 NULLIF(count(*) FILTER (WHERE tc.result::text IN ('PASS', 'FAIL')), 0) * 100, 2)
                             FROM testcase tc JOIN test_executions te ON te."testId" = tc."executionId"
                             WHERE te."buildId" = b.build_id), 0),
                         'averageExecutionTime', COALESCE((SELECT round(avg(GREATEST(0,
                             extract(epoch FROM (COALESCE("endedAt", now()) -
                                 COALESCE("startedAt", "createdAt"))) * 1000))::numeric, 2)
                             FROM test_executions WHERE "buildId" = b.build_id), 0),
                         'uniqueDeviceCount', (SELECT count(DISTINCT "targetDeviceId")
                             FROM device_action_queue WHERE "releaseId"::text = b.build_id
                                 AND "targetDeviceId" IS NOT NULL),
                         'executionCount', (SELECT count(*) FROM test_executions
                             WHERE "buildId" = b.build_id))
                     FROM builds b JOIN releases r ON r.id::text = b.build_id
                     ORDER BY b.last_used DESC"#,
        )
        .bind(count)
        .fetch_all(pool)
        .await?)
}

pub(crate) async fn build_performance_by_id(
    pool: &PgPool,
    build_id: &str,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
                r#"SELECT jsonb_build_object(
                         'buildId', $1::text, 'buildVersion', COALESCE(r.version,
                             (SELECT "buildVersion" FROM test_executions WHERE "buildId" = $1
                                 ORDER BY "createdAt" DESC LIMIT 1)),
                         'flagged', COALESCE(r."isFaulty", false), 'deviceType', r."deviceType",
                         'status', COALESCE(r.status::text, (SELECT status::text FROM test_executions
                             WHERE "buildId" = $1 ORDER BY "createdAt" DESC LIMIT 1)),
                         'lastTestExecutionId', (SELECT "testId" FROM test_executions
                             WHERE "buildId" = $1 ORDER BY "createdAt" DESC LIMIT 1),
                         'passedTestCaseCount', (SELECT count(*) FROM testcase tc JOIN test_executions te
                             ON te."testId" = tc."executionId" WHERE te."buildId" = $1
                                 AND tc.result::text = 'PASS'),
                         'failedTestCaseCount', (SELECT count(*) FROM testcase tc JOIN test_executions te
                             ON te."testId" = tc."executionId" WHERE te."buildId" = $1
                                 AND tc.result::text = 'FAIL'),
                         'passedTestCasePercentage', COALESCE((SELECT round(
                             count(*) FILTER (WHERE tc.result::text = 'PASS')::numeric /
                                 NULLIF(count(*) FILTER (WHERE tc.result::text IN ('PASS', 'FAIL')), 0) * 100, 2)
                             FROM testcase tc JOIN test_executions te ON te."testId" = tc."executionId"
                             WHERE te."buildId" = $1), 0),
                         'averageExecutionTime', COALESCE((SELECT round(avg(GREATEST(0,
                             extract(epoch FROM (COALESCE("endedAt", now()) -
                                 COALESCE("startedAt", "createdAt"))) * 1000))::numeric, 2)
                             FROM test_executions WHERE "buildId" = $1), 0),
                         'uniqueDeviceCount', (SELECT count(DISTINCT "targetDeviceId")
                             FROM device_action_queue WHERE "releaseId"::text = $1
                                 AND "targetDeviceId" IS NOT NULL),
                         'executionCount', (SELECT count(*) FROM test_executions WHERE "buildId" = $1))
                     FROM (SELECT 1) seed LEFT JOIN releases r ON r.id::text = $1"#,
        )
        .bind(build_id)
        .fetch_one(pool)
        .await?)
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

pub(crate) async fn test_execution_analytics(
    pool: &PgPool,
    user_id: &str,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    let rows = sqlx::query_as::<_, TestCaseAnalyticsRow>(
        r#"SELECT te."createdAt" AS created_at, tc.result::text AS result
               FROM test_executions te JOIN testcase tc ON tc."executionId" = te."testId"
               WHERE te."createdBy" = $1
                 AND te.status::text NOT IN ('cancelled', 'queued', 'not_executed', 'failed')"#,
    )
    .bind(user_id)
    .fetch_all(pool)
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
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
                 'id', te.id, 'testPlanName', te."testPlanName", 'createdAt', te."createdAt",
                 'startedAt', te."startedAt", 'endedAt', te."endedAt", 'status', te.status,
                 'total', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId"),
                 'executed', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId"
                   AND tc.result::text IN ('PASS', 'FAIL')))
               FROM test_executions te WHERE te."createdBy" = $1 AND te.status::text = 'in_progress'
               ORDER BY te."createdAt" DESC"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

pub(crate) async fn test_plan_summary(
    pool: &PgPool,
    user_id: &str,
) -> Result<serde_json::Value, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
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
                   ('completed', 'failed', 'cancelled')))
               FROM test_executions te WHERE te."createdBy" = $1"#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?)
}

pub(crate) async fn daily_test_summary(
    pool: &PgPool,
    user_id: &str,
    days: i64,
) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar(
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
                   WHERE tc."executionId" = te."testId" AND tc.result::text = 'FAIL')), 0))
               FROM days LEFT JOIN test_executions te ON te."createdBy" = $1
                 AND te.status::text <> 'cancelled'
                 AND te."createdAt" >= days.day AND te."createdAt" < days.day + interval '1 day'
               GROUP BY days.day ORDER BY days.day ASC"#,
    )
    .bind(user_id)
    .bind(days)
    .fetch_all(pool)
    .await?)
}

fn aggregate_test_cases<'a>(
    rows: impl Iterator<Item = &'a TestCaseAnalyticsRow>,
) -> serde_json::Value {
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
