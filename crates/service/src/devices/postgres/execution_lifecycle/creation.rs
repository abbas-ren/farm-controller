use super::*;

impl PostgresDeviceRepository {
    pub(super) async fn create_test_execution_inner(
        &self,
        execution: &ExecutionCreation,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let release = sqlx::query_as::<_, (String, bool)>(
            r#"SELECT version, "isFaulty" FROM releases WHERE id::text = $1 FOR SHARE"#,
        )
        .bind(&execution.build_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            DeviceRepositoryError::NotFound(format!("Release {} not found", execution.build_id))
        })?;
        if release.1 {
            return Err(DeviceRepositoryError::Validation(format!(
                "Cannot create test execution: Release {} is marked as faulty",
                execution.build_id
            )));
        }
        let test_id = Uuid::new_v4().simple().to_string();
        let name = execution
            .name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "{} | {}:{} | {}",
                    execution.plan_name,
                    execution.device_type,
                    release.0,
                    chrono::Utc::now().format("%b %-d, %Y %-I:%M %p")
                )
            });
        let test_suites = serde_json::to_value(&execution.test_suites)
            .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
        let selection = execution
            .selection
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
        sqlx::query(
            r#"INSERT INTO test_executions
               ("testId", "deviceFamily", "deviceType", "deviceId", "buildId", "buildVersion",
                "testPlanId", "testSuits", "selectionInput", name, status, "createdBy",
                "testPlanName", "isAllSelected", "executionPhase", logs, "createdAt", "updatedAt")
               VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8, $9, 'not_executed', $10,
                       $11, $12, 'PREPARE_ARTIFACTS', '[]'::jsonb, now(), now())"#,
        )
        .bind(&test_id)
        .bind(&execution.device_family)
        .bind(&execution.device_type)
        .bind(&execution.build_id)
        .bind(&release.0)
        .bind(execution.plan_id)
        .bind(test_suites)
        .bind(selection)
        .bind(name)
        .bind(&execution.user_id)
        .bind(&execution.plan_name)
        .bind(execution.is_all_selected)
        .execute(&mut *transaction)
        .await?;
        for case in &execution.cases {
            let result = case
                .result
                .as_deref()
                .filter(|value| matches!(*value, "PASS" | "FAIL"));
            sqlx::query(
                r#"INSERT INTO testcase
                   ("testCaseId", title, "executionId", result, "suiteId", "scriptFile", "planId",
                    "priorityId", "order", "suiteName", labels, "preCondition", "createdAt", "updatedAt")
                   VALUES ($1, $2, $3, $4::"enum_testcase_result", $5, $6, $7, $8, $9, $10, $11, $12, now(), now())"#,
            )
            .bind(case.test_case_id)
            .bind(&case.title)
            .bind(&test_id)
            .bind(result)
            .bind(case.suite_id)
            .bind(&case.script_file)
            .bind(case.plan_id)
            .bind(case.priority_id)
            .bind(case.order)
            .bind(&case.suite_name)
            .bind(&case.labels)
            .bind(&case.pre_condition)
            .execute(&mut *transaction)
            .await?;
        }
        let created = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(te) || jsonb_build_object(
                 'Device', NULL,
                 'testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc."suiteId", tc.id)
                                        FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb),
                 'logs', '[]'::jsonb)
               FROM test_executions te WHERE te."testId" = $1"#,
        )
        .bind(&test_id)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(created)
    }

    pub(super) async fn test_execution_build_version_inner(
        &self,
        test_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT "buildVersion" FROM test_executions WHERE "testId" = $1"#,
        )
        .bind(test_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    pub(super) async fn executions_by_build_inner(
        &self,
        build_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let total = sqlx::query_scalar::<_, i64>(
            r#"SELECT count(*) FROM test_executions WHERE "buildId" = $1"#,
        )
        .bind(build_id)
        .fetch_one(&self.pool)
        .await?;
        let executions = sqlx::query_scalar::<_, serde_json::Value>(r#"SELECT to_jsonb(te) || jsonb_build_object('testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id) FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb)) FROM test_executions te WHERE te."buildId" = $1 ORDER BY te."createdAt" DESC LIMIT $2 OFFSET $3"#).bind(build_id).bind(limit).bind(offset).fetch_all(&self.pool).await?;
        Ok(
            serde_json::json!({"limit": limit, "offset": offset, "total": total, "executions": executions}),
        )
    }

    pub(super) async fn test_results_inner(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(r#"SELECT jsonb_build_object('status', te.status, 'testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id) FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb)) FROM test_executions te WHERE te."testId" = $1"#).bind(test_id).fetch_optional(&self.pool).await?)
    }
}
