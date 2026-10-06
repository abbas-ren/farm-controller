use super::*;

#[async_trait]
impl ControllerRepository for PostgresDeviceRepository {
    async fn device_action_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceActionTarget>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, (String, String, String)>(
            r#"SELECT "deviceId", "ipAddress", status::text
               FROM devices WHERE "deviceId" = $1 AND "deletedAt" IS NULL"#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?
        .map(|(device_id, ip_address, status)| DeviceActionTarget {
            device_id,
            ip_address,
            status,
        }))
    }

    async fn apply_device_action(
        &self,
        device_id: &str,
        action: DeviceAction,
        user_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let device = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(d) FROM devices d
               WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL FOR UPDATE"#,
        )
        .bind(device_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(mut device) = device else {
            transaction.rollback().await?;
            return Ok(None);
        };
        match action {
            DeviceAction::Declined => {
                sqlx::query(
                    r#"INSERT INTO log_entries
                          (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
                       VALUES ('device', $1,
                               jsonb_build_object('action', 'declined', 'deviceData', $2,
                                                  'timestamp', now()),
                               'info', now(), now(), now())"#,
                )
                .bind(device_id)
                .bind(&device)
                .execute(&mut *transaction)
                .await?;
                sqlx::query(r#"DELETE FROM device_interfaces WHERE "deviceId" = $1"#)
                    .bind(device_id)
                    .execute(&mut *transaction)
                    .await?;
                sqlx::query(r#"DELETE FROM devices WHERE "deviceId" = $1"#)
                    .bind(device_id)
                    .execute(&mut *transaction)
                    .await?;
            }
            DeviceAction::Approved => {
                let updated = sqlx::query_scalar::<_, serde_json::Value>(
                    r#"UPDATE devices SET status = 'approved', state = 'free',
                                                     "updatedBy" = $2,
                                                     "updatedAt" = now(), "stateUpdatedAt" = now()
                       WHERE "deviceId" = $1 AND status::text = 'requested'
                         AND "deletedAt" IS NULL
                       RETURNING to_jsonb(devices)"#,
                )
                .bind(device_id)
                .bind(user_id)
                .fetch_optional(&mut *transaction)
                .await?;
                let Some(updated) = updated else {
                    return Err(DeviceRepositoryError::Validation(
                        "Device is no longer awaiting approval".to_owned(),
                    ));
                };
                device = updated;
                sqlx::query(
                    r#"INSERT INTO device_state_change
                          ("deviceId", state, "changedAt", "createdAt", "updatedAt")
                       VALUES ($1, 'free', now(), now(), now())"#,
                )
                .bind(device_id)
                .execute(&mut *transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        Ok(Some(device))
    }

    async fn list_controllers(
        &self,
        query: &ControllerListQuery,
    ) -> Result<ControllerList, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(20).clamp(1, MAX_PAGE_SIZE);
        let offset = i64::from(page.saturating_sub(1)) * i64::from(limit);
        let search = query.search.as_deref().unwrap_or("").trim();
        let status = query.status.as_deref().unwrap_or("").trim();
        let controller_state = query.state.as_deref().unwrap_or("").trim();
        let sort_by = match query.sort_by.as_deref() {
            Some(
                "updatedAt" | "name" | "deviceFamily" | "macAddress" | "ipAddress" | "status"
                | "state",
            ) => query.sort_by.as_deref().unwrap_or("createdAt"),
            _ => "createdAt",
        };
        let descending = query.desc.unwrap_or(true);
        let total_count: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM device_controllers dc
               WHERE dc."deletedAt" IS NULL
                 AND ($1 = '' OR lower($1) = 'all' OR dc.status::text ILIKE '%' || $1 || '%')
                 AND ($2 = '' OR lower($2) = 'all' OR dc.state::text ILIKE '%' || $2 || '%')
                 AND ($3 = '' OR dc."deviceControllerId" ILIKE '%' || $3 || '%'
                    OR dc.name ILIKE '%' || $3 || '%' OR dc."macAddress" ILIKE '%' || $3 || '%'
                    OR dc."ipAddress" ILIKE '%' || $3 || '%')"#,
        )
        .bind(status)
        .bind(controller_state)
        .bind(search)
        .fetch_one(&self.pool)
        .await?;
        let data = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(dc) || jsonb_build_object('relays', COALESCE((
                    SELECT jsonb_agg(to_jsonb(r) || jsonb_build_object('relayChannels', COALESCE((
                        SELECT jsonb_agg(to_jsonb(rc) || jsonb_build_object('device', CASE
                            WHEN d."deviceId" IS NULL THEN NULL ELSE jsonb_build_object(
                                'deviceId', d."deviceId", 'macAddress', d."macAddress",
                                'deviceName', d."deviceName", 'deviceType', d."deviceType",
                                'ipAddress', d."ipAddress") END) ORDER BY rc."channelNumber")
                        FROM relay_channels rc LEFT JOIN devices d ON d."deviceId" = rc."deviceId"
                        WHERE rc."relayId" = r.id AND rc."deletedAt" IS NULL
                    ), '[]'::jsonb)) ORDER BY r."createdAt")
                    FROM relays r WHERE r."deviceControllerId" = dc."deviceControllerId"
                      AND r."deletedAt" IS NULL
                ), '[]'::jsonb))
               FROM device_controllers dc
               WHERE dc."deletedAt" IS NULL
                 AND ($1 = '' OR lower($1) = 'all' OR dc.status::text ILIKE '%' || $1 || '%')
                 AND ($2 = '' OR lower($2) = 'all' OR dc.state::text ILIKE '%' || $2 || '%')
                 AND ($3 = '' OR dc."deviceControllerId" ILIKE '%' || $3 || '%'
                    OR dc.name ILIKE '%' || $3 || '%' OR dc."macAddress" ILIKE '%' || $3 || '%'
                    OR dc."ipAddress" ILIKE '%' || $3 || '%')
               ORDER BY
                 CASE WHEN $4 = 'createdAt' AND $5 THEN dc."createdAt" END DESC,
                 CASE WHEN $4 = 'createdAt' AND NOT $5 THEN dc."createdAt" END ASC,
                 CASE WHEN $4 = 'updatedAt' AND $5 THEN dc."updatedAt" END DESC,
                 CASE WHEN $4 = 'updatedAt' AND NOT $5 THEN dc."updatedAt" END ASC,
                 CASE WHEN $4 = 'name' AND $5 THEN dc.name END DESC,
                 CASE WHEN $4 = 'name' AND NOT $5 THEN dc.name END ASC,
                 CASE WHEN $4 = 'deviceFamily' AND $5 THEN dc."deviceFamily" END DESC,
                 CASE WHEN $4 = 'deviceFamily' AND NOT $5 THEN dc."deviceFamily" END ASC,
                 CASE WHEN $4 = 'macAddress' AND $5 THEN dc."macAddress" END DESC,
                 CASE WHEN $4 = 'macAddress' AND NOT $5 THEN dc."macAddress" END ASC,
                 CASE WHEN $4 = 'ipAddress' AND $5 THEN dc."ipAddress" END DESC,
                 CASE WHEN $4 = 'ipAddress' AND NOT $5 THEN dc."ipAddress" END ASC,
                 CASE WHEN $4 = 'status' AND $5 THEN dc.status::text END DESC,
                 CASE WHEN $4 = 'status' AND NOT $5 THEN dc.status::text END ASC,
                 CASE WHEN $4 = 'state' AND $5 THEN dc.state::text END DESC,
                 CASE WHEN $4 = 'state' AND NOT $5 THEN dc.state::text END ASC
               LIMIT $6 OFFSET $7"#,
        )
        .bind(status)
        .bind(controller_state)
        .bind(search)
        .bind(sort_by)
        .bind(descending)
        .bind(i64::from(limit))
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let controller_counts: (i64, i64, i64) = sqlx::query_as(
            r#"SELECT count(*), count(*) FILTER (WHERE state::text = 'active'),
               count(*) FILTER (WHERE state::text IN ('not-reachable', 'not_reachable'))
               FROM device_controllers WHERE "deletedAt" IS NULL"#,
        )
        .fetch_one(&self.pool)
        .await?;
        let relay_counts: (i64, i64) = sqlx::query_as(
            r#"SELECT count(*), count(*) FILTER (WHERE state::text = 'connected')
               FROM relays WHERE "deletedAt" IS NULL"#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(ControllerList {
            data,
            total_pages: u64::try_from(total_count)
                .unwrap_or(0)
                .div_ceil(u64::from(limit)),
            current_page: page,
            total_count: u64::try_from(total_count).unwrap_or(0),
            summary: serde_json::json!({
                "controllers": {"total": controller_counts.0, "active": controller_counts.1, "notReachable": controller_counts.2},
                "relays": {"total": relay_counts.0, "connected": relay_counts.1, "disconnected": (relay_counts.0 - relay_counts.1).max(0)}
            }),
        })
    }

    async fn edit_controller(
        &self,
        controller_id: &str,
        request: &ControllerEditRequest,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        controller_store::edit(&self.pool, controller_id, request).await
    }

    async fn controller_delete_target(
        &self,
        controller_id: &str,
    ) -> Result<Option<ControllerDeleteTarget>, DeviceRepositoryError> {
        controller_store::delete_target(&self.pool, controller_id).await
    }

    async fn delete_controller(&self, controller_id: &str) -> Result<bool, DeviceRepositoryError> {
        controller_store::delete(&self.pool, controller_id).await
    }

    async fn device_delete_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceDeleteTarget>, DeviceRepositoryError> {
        Ok(sqlx::query_as::<_, DeviceDeleteTarget>(
            r#"SELECT d."ipAddress" AS ip_address, d."deviceFamily" AS device_family,
                 COALESCE(r."deviceControllerId", d."controllerId") AS controller_id,
                 dc."ipAddress" AS controller_ip, r."serialNumber" AS relay_serial,
                 rc."channelNumber" AS relay_channel
               FROM devices d
               LEFT JOIN relay_channels rc ON rc."deviceId" = d."deviceId" AND rc."deletedAt" IS NULL
               LEFT JOIN relays r ON r.id = rc."relayId" AND r."deletedAt" IS NULL
               LEFT JOIN device_controllers dc ON dc."deviceControllerId" = COALESCE(r."deviceControllerId", d."controllerId")
               WHERE d."deviceId" = $1 AND d."deletedAt" IS NULL
               ORDER BY r."updatedAt" DESC NULLS LAST LIMIT 1"#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn delete_device(&self, device_id: &str) -> Result<bool, DeviceRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            r#"UPDATE device_controllers controller
               SET mappings = COALESCE((
                       SELECT jsonb_object_agg(entry.key, entry.value)
                       FROM jsonb_each(COALESCE(controller.mappings, '{}'::jsonb)) entry
                       WHERE regexp_replace(lower(entry.key), '[:-]', '', 'g') <> $1
                   ), '{}'::jsonb),
                   "updatedAt" = now()
               WHERE EXISTS (
                   SELECT 1
                   FROM jsonb_each(COALESCE(controller.mappings, '{}'::jsonb)) entry
                   WHERE regexp_replace(lower(entry.key), '[:-]', '', 'g') = $1
               )"#,
        )
        .bind(device_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(r#"DELETE FROM device_interfaces WHERE "deviceId" = $1"#)
            .bind(device_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(r#"DELETE FROM relay_channels WHERE "deviceId" = $1"#)
            .bind(device_id)
            .execute(&mut *transaction)
            .await?;
        let deleted = sqlx::query(r#"DELETE FROM devices WHERE "deviceId" = $1"#)
            .bind(device_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            > 0;
        transaction.commit().await?;
        Ok(deleted)
    }
}
