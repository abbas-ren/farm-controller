use super::*;

#[async_trait]
impl ExecutionQueryRepository for PostgresDeviceRepository {
    async fn list_test_executions(
        &self,
        user_id: &str,
        query: &ExecutionListQuery,
    ) -> Result<ExecutionList, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(10).max(1);
        let search = query.search.as_deref().unwrap_or_default();
        let sort_by = query
            .sort_by
            .as_deref()
            .filter(|value| {
                matches!(
                    *value,
                    "createdAt" | "status" | "testPlanName" | "buildVersion" | "deviceType"
                )
            })
            .unwrap_or("createdAt");
        let descending = query.desc.as_deref().unwrap_or("true") == "true";
        let total = sqlx::query_scalar::<_, i64>(
            r#"SELECT count(*) FROM test_executions te LEFT JOIN devices d ON d."deviceId" = te."deviceId"
               WHERE te."createdBy" = $1 AND ($2 = '' OR d."deviceType" ILIKE '%' || $2 || '%')"#,
        ).bind(user_id).bind(search).fetch_one(&self.pool).await?;
        let data = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT jsonb_build_object(
                 'testId', te."testId", 'durationSeconds', GREATEST(0, floor(extract(epoch FROM (COALESCE(te."endedAt", now()) - COALESCE(te."startedAt", now())))))::bigint,
                 'total', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId"),
                 'passed', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId" AND tc.result::text = 'PASS'),
                 'failed', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId" AND tc.result::text = 'FAIL'),
                 'buildId', te."buildId", 'buildVersion', te."buildVersion", 'deviceName', COALESCE(d."deviceType", ''),
                 'deviceId', COALESCE(te."deviceId", ''), 'status', te.status, 'testCases', '{}'::jsonb,
                 'testPlanName', te."testPlanName", 'logs', '[]'::jsonb, 'testCycleId', COALESCE(te."testCycleId", ''),
                 'createdAt', te."createdAt", 'rtosLogPath', te."rtosLogPath")
               FROM test_executions te LEFT JOIN devices d ON d."deviceId" = te."deviceId"
               WHERE te."createdBy" = $1 AND ($2 = '' OR d."deviceType" ILIKE '%' || $2 || '%')
               ORDER BY CASE WHEN $3 = 'createdAt' AND $4 THEN te."createdAt" END DESC,
                 CASE WHEN $3 = 'createdAt' AND NOT $4 THEN te."createdAt" END ASC,
                 CASE WHEN $3 = 'status' AND $4 THEN te.status::text END DESC,
                 CASE WHEN $3 = 'status' AND NOT $4 THEN te.status::text END ASC,
                 CASE WHEN $3 = 'testPlanName' AND $4 THEN te."testPlanName" END DESC,
                 CASE WHEN $3 = 'testPlanName' AND NOT $4 THEN te."testPlanName" END ASC,
                 CASE WHEN $3 = 'buildVersion' AND $4 THEN te."buildVersion" END DESC,
                 CASE WHEN $3 = 'buildVersion' AND NOT $4 THEN te."buildVersion" END ASC,
                 CASE WHEN $3 = 'deviceType' AND $4 THEN d."deviceType" END DESC,
                 CASE WHEN $3 = 'deviceType' AND NOT $4 THEN d."deviceType" END ASC
               LIMIT $5 OFFSET $6"#,
        ).bind(user_id).bind(search).bind(sort_by).bind(descending).bind(limit).bind((page - 1) * limit).fetch_all(&self.pool).await?;
        Ok(ExecutionList {
            data,
            total: total as u64,
            current_page: page as u64,
            total_pages: ((total + limit - 1) / limit).max(0) as u64,
        })
    }

    async fn test_execution_by_device(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(te) FROM test_executions te
               WHERE te."deviceId" = $1 AND te."createdBy" = $2
               ORDER BY te."updatedAt" DESC LIMIT 1"#,
        )
        .bind(device_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn in_progress_test_executions(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT jsonb_build_object('deviceId', "deviceId", 'status', status, 'testId', "testId")
               FROM test_executions WHERE "createdBy" = $1
                 AND status::text IN ('in_progress', 'not_executed', 'queued')"#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn single_test_execution(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(te) || jsonb_build_object('testCases', COALESCE((
                   SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id) FROM testcase tc
                   WHERE tc."executionId" = te."testId"), '[]'::jsonb))
               FROM test_executions te WHERE te."testId" = $1"#,
        )
        .bind(test_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn execution_case_ids(
        &self,
        test_id: &str,
    ) -> Result<Option<Vec<String>>, DeviceRepositoryError> {
        let exists = sqlx::query_scalar::<_, bool>(
            r#"SELECT EXISTS(SELECT 1 FROM test_executions
               WHERE "testId" = $1 AND status::text NOT IN ('cancelled', 'failed'))"#,
        )
        .bind(test_id)
        .fetch_one(&self.pool)
        .await?;
        if !exists {
            return Ok(None);
        }
        Ok(Some(
            sqlx::query_scalar::<_, String>(
                r#"SELECT "testCaseId"::text FROM testcase
               WHERE "executionId" = $1 ORDER BY "suiteId", id"#,
            )
            .bind(test_id)
            .fetch_all(&self.pool)
            .await?,
        ))
    }

    async fn test_execution(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(te) || jsonb_build_object('Device', CASE WHEN d."deviceId" IS NULL THEN NULL ELSE to_jsonb(d) END)
               FROM test_executions te LEFT JOIN devices d ON d."deviceId" = te."deviceId"
               WHERE te."createdBy" = $1
                 AND (($2 = 'latest' AND te.status::text IN ('not_executed', 'queued', 'in_progress')) OR te."testId" = $2)
               ORDER BY CASE WHEN $2 = 'latest' THEN te."createdAt" END DESC LIMIT 1"#,
        )
        .bind(user_id)
        .bind(execution_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn test_execution_summary(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let Some(mut summary) = sqlx::query_scalar::<_, serde_json::Value>(
                        r#"SELECT jsonb_build_object(
                                 'testId', te."testId",
                                 'durationSeconds', GREATEST(0, floor(extract(epoch FROM (COALESCE(te."endedAt", now()) - COALESCE(te."startedAt", now())))))::bigint,
                                 'total', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId"),
                                 'passed', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId" AND tc.result::text = 'PASS'),
                                 'failed', (SELECT count(*) FROM testcase tc WHERE tc."executionId" = te."testId" AND tc.result::text = 'FAIL'),
                                 'buildId', te."buildId", 'buildVersion', te."buildVersion",
                                 'deviceName', COALESCE(d."deviceType", ''), 'deviceType', COALESCE(d."deviceType", ''),
                                 'status', te.status,
                                 'testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id) FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb),
                                 'testPlanName', te."testPlanName", 'testCycleId', te."testCycleId",
                                 'createdAt', te."createdAt", 'rtosLogPath', te."rtosLogPath")
                             FROM test_executions te
                             LEFT JOIN devices d ON d."deviceId" = te."deviceId"
                             WHERE te."testId" = $1"#,
                )
                .bind(execution_id)
                .fetch_optional(&self.pool)
                .await?
                else {
                        return Ok(None);
                };
        let analytics = sqlx::query_scalar::<_, serde_json::Value>(
                        r#"WITH eligible AS (
                                 SELECT te."createdAt", tc.result::text AS result
                                 FROM test_executions te
                                 JOIN testcase tc ON tc."executionId" = te."testId"
                                 WHERE te."createdBy" = $1
                                     AND te.status::text NOT IN ('cancelled', 'queued', 'not_executed', 'failed')
                             )
                             SELECT jsonb_build_object(
                                 'allTime', jsonb_build_object(
                                     'total', count(*), 'passed', count(*) FILTER (WHERE result = 'PASS'),
                                     'failed', count(*) FILTER (WHERE result = 'FAIL'),
                                     'inProgress', count(*) FILTER (WHERE result IS NULL OR result NOT IN ('PASS', 'FAIL'))),
                                 'currentWeek', jsonb_build_object(
                                     'week', to_char(date_trunc('week', now()), 'DD Mon') || ' - ' || to_char(date_trunc('week', now()) + interval '6 days', 'DD Mon'),
                                     'total', count(*) FILTER (WHERE "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week'),
                                     'passed', count(*) FILTER (WHERE result = 'PASS' AND "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week'),
                                     'failed', count(*) FILTER (WHERE result = 'FAIL' AND "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week'),
                                     'inProgress', count(*) FILTER (WHERE (result IS NULL OR result NOT IN ('PASS', 'FAIL')) AND "createdAt" >= date_trunc('week', now()) AND "createdAt" < date_trunc('week', now()) + interval '1 week')),
                                 'currentMonth', jsonb_build_object(
                                     'month', to_char(date_trunc('month', now()), 'Mon YYYY'),
                                     'total', count(*) FILTER (WHERE "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month'),
                                     'passed', count(*) FILTER (WHERE result = 'PASS' AND "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month'),
                                     'failed', count(*) FILTER (WHERE result = 'FAIL' AND "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month'),
                                     'inProgress', count(*) FILTER (WHERE (result IS NULL OR result NOT IN ('PASS', 'FAIL')) AND "createdAt" >= date_trunc('month', now()) AND "createdAt" < date_trunc('month', now()) + interval '1 month')))
                             FROM eligible"#,
                )
                .bind(user_id)
                .fetch_one(&self.pool)
                .await?;
        summary["analytics"] = analytics;
        Ok(Some(summary))
    }

    async fn test_export_rows(
        &self,
        user_id: &str,
    ) -> Result<Vec<TestExportRow>, DeviceRepositoryError> {
        test_export::rows(&self.pool, user_id).await
    }
}
