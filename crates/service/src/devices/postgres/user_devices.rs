use super::*;

#[async_trait]
impl UserDeviceRepository for PostgresDeviceRepository {
    async fn list_user_devices(
        &self,
        query: &UserDeviceListQuery,
    ) -> Result<DeviceList, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(10).clamp(1, MAX_PAGE_SIZE);
        let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
        let search = query.search.as_deref().unwrap_or("").trim();
        let family = query.device_family.as_deref().unwrap_or("").trim();
        let state_filter = query.filter_by.as_deref().unwrap_or("").trim();
        let show_all = query.show_all.unwrap_or(false);
        let sort_by = match query.sort_by.as_deref() {
            Some(
                "updatedAt" | "deviceName" | "status" | "softwareVersion" | "lastTestExecution",
            ) => query.sort_by.as_deref().unwrap_or("createdAt"),
            _ => "createdAt",
        };
        let descending = query.desc.unwrap_or(true);
        let total_devices: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM devices d WHERE d."deletedAt" IS NULL
               AND d.status::text = 'approved'
               AND ($1 = '' OR upper($1) = 'ALL' OR d."deviceFamily" ILIKE '%' || $1 || '%')
               AND ($2 = '' OR lower($2) = 'all' OR d.state::text = $2)
               AND ($3 = '' OR d."deviceName" ILIKE '%' || $3 || '%'
                    OR d."softwareVersion" ILIKE '%' || $3 || '%')
               AND ($4 OR d.state::text IN ('free', 'busy') OR (
                    d."lastTestExecution" IS NOT NULL
                    AND COALESCE(d."lastExecutionStatus"::text, '') NOT IN ('failed', 'completed', 'cancelled')
                    AND d.state::text <> 'unknown'))"#,
        )
        .bind(family).bind(state_filter).bind(search).bind(show_all)
        .fetch_one(&self.pool).await?;
        let data = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(d) || jsonb_build_object(
                'interfaces', COALESCE((SELECT jsonb_agg(to_jsonb(di) ORDER BY di.id)
                    FROM device_interfaces di WHERE di."deviceId" = d."deviceId"), '[]'::jsonb),
                'testExecutions', COALESCE((SELECT jsonb_agg(to_jsonb(te) || jsonb_build_object(
                    'testCases', COALESCE((SELECT jsonb_agg(to_jsonb(tc) ORDER BY tc.id)
                        FROM testcase tc WHERE tc."executionId" = te."testId"), '[]'::jsonb)))
                    FROM (SELECT * FROM test_executions WHERE "deviceId" = d."deviceId"
                        ORDER BY "updatedAt" DESC LIMIT 1) te), '[]'::jsonb))
               FROM devices d WHERE d."deletedAt" IS NULL AND d.status::text = 'approved'
               AND ($1 = '' OR upper($1) = 'ALL' OR d."deviceFamily" ILIKE '%' || $1 || '%')
               AND ($2 = '' OR lower($2) = 'all' OR d.state::text = $2)
               AND ($3 = '' OR d."deviceName" ILIKE '%' || $3 || '%'
                    OR d."softwareVersion" ILIKE '%' || $3 || '%')
               AND ($4 OR d.state::text IN ('free', 'busy') OR (
                    d."lastTestExecution" IS NOT NULL
                    AND COALESCE(d."lastExecutionStatus"::text, '') NOT IN ('failed', 'completed', 'cancelled')
                    AND d.state::text <> 'unknown'))
               ORDER BY
                 CASE WHEN $5 IN ('createdAt') AND $6 THEN d."createdAt" END DESC,
                 CASE WHEN $5 IN ('createdAt') AND NOT $6 THEN d."createdAt" END ASC,
                 CASE WHEN $5 IN ('updatedAt', 'lastTestExecution') AND $6 THEN d."updatedAt" END DESC,
                 CASE WHEN $5 IN ('updatedAt', 'lastTestExecution') AND NOT $6 THEN d."updatedAt" END ASC,
                 CASE WHEN $5 = 'deviceName' AND $6 THEN d."deviceName" END DESC,
                 CASE WHEN $5 = 'deviceName' AND NOT $6 THEN d."deviceName" END ASC,
                 CASE WHEN $5 = 'status' AND $6 THEN d.status::text END DESC,
                 CASE WHEN $5 = 'status' AND NOT $6 THEN d.status::text END ASC,
                 CASE WHEN $5 = 'softwareVersion' AND $6 THEN d."softwareVersion" END DESC,
                 CASE WHEN $5 = 'softwareVersion' AND NOT $6 THEN d."softwareVersion" END ASC
               LIMIT $7 OFFSET $8"#,
        )
        .bind(family).bind(state_filter).bind(search).bind(show_all)
        .bind(sort_by).bind(descending).bind(i64::from(limit)).bind(offset)
        .fetch_all(&self.pool).await?;
        Ok(DeviceList {
            data,
            total_pages: u64::try_from(total_devices)
                .unwrap_or(0)
                .div_ceil(u64::from(limit)),
            current_page: page,
            total_devices: u64::try_from(total_devices).unwrap_or(0),
            requested_count: 0,
            device_timers: BTreeMap::new(),
            device_timeouts: BTreeMap::new(),
            state_count: BTreeMap::new(),
        })
    }

    async fn active_devices(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(d) || jsonb_build_object(
                'interfaces', COALESCE((SELECT jsonb_agg(to_jsonb(di) ORDER BY di.id)
                    FROM device_interfaces di WHERE di."deviceId" = d."deviceId"), '[]'::jsonb))
               FROM devices d
               WHERE d."deletedAt" IS NULL AND d.status::text = 'approved'
               ORDER BY d."createdAt" DESC"#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn update_heartbeat_timeout(
        &self,
        device_id: &str,
        value: i64,
    ) -> Result<bool, DeviceRepositoryError> {
        Ok(sqlx::query(
            r#"UPDATE devices SET "heartbeatTimer" = $2, "updatedAt" = now()
               WHERE "deviceId" = $1 AND "deletedAt" IS NULL"#,
        )
        .bind(device_id)
        .bind(value)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }
}
