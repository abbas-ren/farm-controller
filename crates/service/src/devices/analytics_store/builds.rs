use sqlx::PgPool;

use super::super::error::DeviceRepositoryError;

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
