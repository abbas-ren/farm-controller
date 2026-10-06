use super::*;

#[async_trait]
impl DeviceInventoryRepository for PostgresDeviceRepository {
    async fn device_families(&self) -> Result<Vec<String>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, String>(
            r#"
            SELECT DISTINCT "deviceFamily"
            FROM devices
            WHERE "deletedAt" IS NULL AND "deviceFamily" IS NOT NULL
            ORDER BY "deviceFamily"
            "#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn device_types(
        &self,
        device_family: Option<&str>,
    ) -> Result<Vec<String>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, String>(
            r#"
            SELECT DISTINCT "deviceType"
            FROM devices
            WHERE "deletedAt" IS NULL AND "deviceType" IS NOT NULL
              AND ($1::text IS NULL OR $1 = 'ALL' OR "deviceFamily" = $1)
            ORDER BY "deviceType"
            "#,
        )
        .bind(device_family)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn device_by_id(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT to_jsonb(device_row) || jsonb_build_object(
                'interfaces', COALESCE((
                    SELECT jsonb_agg(to_jsonb(interface_row) ORDER BY interface_row.id)
                    FROM device_interfaces interface_row
                    WHERE interface_row."deviceId" = device_row."deviceId"
                ), '[]'::jsonb),
                'testExecutions', COALESCE((
                    SELECT jsonb_agg(to_jsonb(execution_row))
                    FROM (
                        SELECT * FROM test_executions
                        WHERE "deviceId" = device_row."deviceId"
                        ORDER BY "updatedAt" DESC LIMIT 1
                    ) execution_row
                ), '[]'::jsonb)
            )
            FROM devices device_row
            WHERE device_row."deviceId" = $1 AND device_row."deletedAt" IS NULL
            "#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn latest_heartbeat(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let heartbeat = sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT jsonb_build_object(
                'timestamp', timestamp,
                'data', data,
                'timeout', timeout
            )
            FROM heartbeats
            WHERE "deviceId" = $1
              AND NOT (data @> '{"status":"disconnected"}'::jsonb)
            ORDER BY timestamp DESC LIMIT 1
            "#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?;
        if heartbeat.is_some() {
            return Ok(heartbeat);
        }
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT jsonb_build_object(
                'timestamp', timestamp,
                'data', data,
                'timeout', timeout
            )
            FROM device_controller_heartbeats
            WHERE "controllerId" = $1
              AND NOT (data @> '{"status":"disconnected"}'::jsonb)
            ORDER BY timestamp DESC LIMIT 1
            "#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn topology(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT jsonb_build_object(
                'deviceId', device."deviceId",
                'deviceName', device."deviceName",
                'deviceType', device."deviceType",
                'deviceFamily', device."deviceFamily",
                'macAddress', device."macAddress",
                'ipAddress', device."ipAddress",
                'state', device.state,
                'status', device.status,
                'controllerId', COALESCE((
                    SELECT relay."deviceControllerId"
                    FROM relay_channels channel
                    JOIN relays relay ON relay.id = channel."relayId"
                    WHERE channel."deviceId" = device."deviceId"
                      AND channel."deletedAt" IS NULL
                      AND relay."deletedAt" IS NULL
                    ORDER BY relay."updatedAt" DESC LIMIT 1
                ), device."controllerId"),
                'heartbeatTimer', device."heartbeatTimer"
            )
            FROM devices device
            WHERE device.status = 'approved' AND device."deletedAt" IS NULL
            ORDER BY device."deviceName" NULLS LAST, device."deviceId"
            "#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn list_devices(
        &self,
        query: &DeviceListQuery,
        is_admin: bool,
    ) -> Result<DeviceList, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(16).clamp(1, MAX_PAGE_SIZE);
        let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
        let search = query.search.as_deref().unwrap_or("").trim();
        let filter_by = match query.filter_by.as_deref().unwrap_or("") {
            "deviceType" | "deviceFamily" | "status" | "state" => {
                query.filter_by.as_deref().unwrap_or("")
            }
            _ => "",
        };
        let filter = query.filter.as_deref().unwrap_or("").trim();
        let sort_by = match query.sort_by.as_deref() {
            Some("updatedAt" | "deviceName" | "status") => {
                query.sort_by.as_deref().unwrap_or("createdAt")
            }
            _ => "createdAt",
        };
        let descending = query.desc.unwrap_or(true);
        let total_devices: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM devices d WHERE
            d."deletedAt" IS NULL
            AND ($1 OR d.status::text = 'approved')
            AND ($2 = '' OR d."deviceName" ILIKE '%' || $2 || '%'
                OR d."deviceId" ILIKE '%' || $2 || '%'
                OR d."deviceType" ILIKE '%' || $2 || '%'
                OR d."macAddress" ILIKE '%' || $2 || '%')
            AND ($3 = '' OR lower($4) = 'all' OR CASE $3
                WHEN 'deviceType' THEN d."deviceType" ILIKE '%' || $4 || '%'
                WHEN 'deviceFamily' THEN d."deviceFamily" ILIKE '%' || $4 || '%'
                WHEN 'status' THEN d.status::text ILIKE '%' || $4 || '%'
                WHEN 'state' THEN d.state::text ILIKE '%' || $4 || '%'
                ELSE TRUE END)
            "#,
        )
        .bind(is_admin)
        .bind(search)
        .bind(filter_by)
        .bind(filter)
        .fetch_one(&self.pool)
        .await?;
        let rows = sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT to_jsonb(d) || jsonb_build_object(
                'interfaces', COALESCE((
                    SELECT jsonb_agg(to_jsonb(di) ORDER BY di.id)
                    FROM device_interfaces di WHERE di."deviceId" = d."deviceId"
                ), '[]'::jsonb),
                'timeout', COALESCE((
                    SELECT h.timeout FROM heartbeats h
                    WHERE h."deviceId" = d."deviceId"
                    ORDER BY h.timestamp DESC LIMIT 1
                ), 0),
                'controllerId', COALESCE((
                    SELECT r."deviceControllerId" FROM relay_channels rc
                    JOIN relays r ON r.id = rc."relayId"
                    WHERE rc."deviceId" = d."deviceId"
                      AND rc."deletedAt" IS NULL AND r."deletedAt" IS NULL
                    LIMIT 1
                ), d."controllerId"),
                'usage', jsonb_build_object('totalHours', 0, 'states', '{{}}'::jsonb)
            )
            FROM devices d WHERE
                d."deletedAt" IS NULL
                AND ($1 OR d.status::text = 'approved')
                AND ($2 = '' OR d."deviceName" ILIKE '%' || $2 || '%'
                    OR d."deviceId" ILIKE '%' || $2 || '%'
                    OR d."deviceType" ILIKE '%' || $2 || '%'
                    OR d."macAddress" ILIKE '%' || $2 || '%')
                AND ($3 = '' OR lower($4) = 'all' OR CASE $3
                    WHEN 'deviceType' THEN d."deviceType" ILIKE '%' || $4 || '%'
                    WHEN 'deviceFamily' THEN d."deviceFamily" ILIKE '%' || $4 || '%'
                    WHEN 'status' THEN d.status::text ILIKE '%' || $4 || '%'
                    WHEN 'state' THEN d.state::text ILIKE '%' || $4 || '%'
                    ELSE TRUE END)
            ORDER BY CASE WHEN d.status::text = 'requested' THEN 0 ELSE 1 END,
                CASE WHEN $5 = 'createdAt' AND $6 THEN d."createdAt" END DESC,
                CASE WHEN $5 = 'createdAt' AND NOT $6 THEN d."createdAt" END ASC,
                CASE WHEN $5 = 'updatedAt' AND $6 THEN d."updatedAt" END DESC,
                CASE WHEN $5 = 'updatedAt' AND NOT $6 THEN d."updatedAt" END ASC,
                CASE WHEN $5 = 'deviceName' AND $6 THEN d."deviceName" END DESC,
                CASE WHEN $5 = 'deviceName' AND NOT $6 THEN d."deviceName" END ASC,
                CASE WHEN $5 = 'status' AND $6 THEN d.status::text END DESC,
                CASE WHEN $5 = 'status' AND NOT $6 THEN d.status::text END ASC
            LIMIT $7 OFFSET $8
            "#,
        )
        .bind(is_admin)
        .bind(search)
        .bind(filter_by)
        .bind(filter)
        .bind(sort_by)
        .bind(descending)
        .bind(i64::from(limit))
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let requested_count: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM devices WHERE "deletedAt" IS NULL AND status::text = 'requested'"#,
        )
        .fetch_one(&self.pool)
        .await?;
        let state_rows: Vec<(String, i64)> = sqlx::query_as(
            r#"SELECT state::text, count(*) FROM devices
               WHERE "deletedAt" IS NULL AND status::text = 'approved' GROUP BY state"#,
        )
        .fetch_all(&self.pool)
        .await?;
        let mut device_timers = BTreeMap::new();
        let mut device_timeouts = BTreeMap::new();
        for row in &rows {
            if let Some(device_id) = row.get("deviceId").and_then(serde_json::Value::as_str) {
                device_timers.insert(
                    device_id.to_owned(),
                    row.get("timeout")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                );
                device_timeouts.insert(
                    device_id.to_owned(),
                    row.get("heartbeatTimer")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                );
            }
        }
        Ok(DeviceList {
            data: rows,
            total_pages: u64::try_from(total_devices)
                .unwrap_or(0)
                .div_ceil(u64::from(limit)),
            current_page: page,
            total_devices: u64::try_from(total_devices).unwrap_or(0),
            requested_count: u64::try_from(requested_count).unwrap_or(0),
            device_timers,
            device_timeouts,
            state_count: state_rows
                .into_iter()
                .map(|(state, count)| (state, u64::try_from(count).unwrap_or(0)))
                .collect(),
        })
    }
}
