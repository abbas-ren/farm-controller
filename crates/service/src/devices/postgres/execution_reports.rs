use super::*;

#[async_trait]
impl ExecutionReportRepository for PostgresDeviceRepository {
    async fn report_generation_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, (String, String)>(r#"SELECT "deviceType", COALESCE("buildVersion", '') FROM test_executions WHERE "testId" = $1 AND status::text = 'completed'"#).bind(test_id).fetch_optional(&self.pool).await?)
    }

    async fn create_execution_report_record(
        &self,
        report_id: Uuid,
        test_id: &str,
        device_type: &str,
        build_version: &str,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let active = sqlx::query_scalar::<_, String>(r#"SELECT status::text FROM execution_reports WHERE "testExecutionId" = $1 AND status::text IN ('generating', 'uploading') LIMIT 1 FOR UPDATE"#).bind(test_id).fetch_optional(&mut *transaction).await?;
        if let Some(status) = active {
            return Err(DeviceRepositoryError::Conflict(format!(
                "A report for this execution is already {status}"
            )));
        }
        sqlx::query(r#"DELETE FROM execution_reports WHERE "testExecutionId" = $1"#)
            .bind(test_id)
            .execute(&mut *transaction)
            .await?;
        let report = sqlx::query_scalar::<_, serde_json::Value>(r#"INSERT INTO execution_reports (id, "testExecutionId", status, "deviceType", "buildVersion", "createdBy", "createdAt", "updatedAt") VALUES ($1, $2, 'generating', $3, $4, $5, now(), now()) RETURNING to_jsonb(execution_reports)"#)
            .bind(report_id).bind(test_id).bind(device_type).bind(build_version).bind(user_id).fetch_one(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(report)
    }

    async fn update_execution_report_status(
        &self,
        report_id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DeviceRepositoryError> {
        sqlx::query(r#"UPDATE execution_reports SET status = $2::"enum_execution_reports_status", "uploadError" = $3, "updatedAt" = now() WHERE id = $1"#).bind(report_id).bind(status).bind(error).execute(&self.pool).await?;
        Ok(())
    }

    async fn execution_report(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(report) FROM execution_reports report
               WHERE "testExecutionId" = $1 ORDER BY "createdAt" DESC LIMIT 1"#,
        )
        .bind(test_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn execution_report_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, (String, String)>(
            r#"SELECT "deviceType", COALESCE("buildVersion", '') FROM test_executions WHERE "testId" = $1"#,
        ).bind(test_id).fetch_optional(&self.pool).await?)
    }

    async fn test_case_log_path(
        &self,
        case_id: i64,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT "outputFilePath" FROM testcase WHERE id = $1"#,
        )
        .bind(case_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    async fn execution_cases(
        &self,
        user_id: &str,
        test_id: &str,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError> {
        let owned = sqlx::query_scalar::<_, bool>(r#"SELECT EXISTS(SELECT 1 FROM test_executions WHERE "testId" = $1 AND "createdBy" = $2)"#).bind(test_id).bind(user_id).fetch_one(&self.pool).await?;
        if !owned {
            return Ok(None);
        }
        Ok(Some(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT jsonb_build_object('testCaseId', "testCaseId", 'suiteId', "suiteId", 'result', result,
                 'comment', comment, 'jiraDefect', "jiraDefect", 'title', title, 'suiteName', "suiteName",
                 'scriptFile', "scriptFile", 'id', id, 'updatedAt', "updatedAt",
                 'outputFilePath', "outputFilePath", 'dmesgFilePath', "dmesgFilePath")
               FROM testcase WHERE "executionId" = $1 ORDER BY id"#,
        ).bind(test_id).fetch_all(&self.pool).await?))
    }

    async fn test_case(
        &self,
        case_id: i64,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(tc) FROM testcase tc WHERE id = $1"#,
        )
        .bind(case_id)
        .fetch_optional(&self.pool)
        .await?)
    }
}
