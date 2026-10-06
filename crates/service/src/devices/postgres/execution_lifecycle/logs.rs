use super::*;

impl PostgresDeviceRepository {
    pub(super) async fn create_log_inner(
        &self,
        request: &LogCreateRequest,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        Ok(sqlx::query_scalar::<_, serde_json::Value>(
            r#"INSERT INTO log_entries (type, "referenceId", data, level, timestamp, "createdAt", "updatedAt")
               VALUES ($1::"enum_log_entries_type", $2, $3, $4::"enum_log_entries_level", COALESCE($5, now()), now(), now())
               RETURNING to_jsonb(log_entries)"#,
        )
        .bind(&request.log_type)
        .bind(&request.reference_id)
        .bind(&request.data)
        .bind(&request.level)
        .bind(request.timestamp)
        .fetch_one(&self.pool)
        .await?)
    }

    pub(super) async fn list_logs_inner(
        &self,
        query: &LogListQuery,
        search: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let page = query.page.unwrap_or(1).max(1);
        let page_size = query
            .page_size
            .unwrap_or(20)
            .clamp(1, i64::from(MAX_PAGE_SIZE));
        let offset = (page - 1) * page_size;
        let sort = match query.sort.as_deref() {
            Some("id") => "id",
            Some("type") => "type",
            Some("referenceId") => "referenceId",
            Some("level") => "level",
            Some("createdAt") => "createdAt",
            Some("updatedAt") => "updatedAt",
            _ => "timestamp",
        };
        let order = if query.order.as_deref() == Some("ASC") {
            "ASC"
        } else {
            "DESC"
        };
        let total = sqlx::query_scalar::<_, i64>(
            r#"SELECT count(*) FROM log_entries
                             WHERE ($1::text IS NULL OR type::text = $1)
                                 AND ($2::text IS NULL OR "referenceId" = $2)
                                 AND ($3::text IS NULL OR level::text = $3)
                                 AND ($4::text IS NULL OR data::text ILIKE '%' || $4 || '%')"#,
        )
        .bind(query.log_type.as_deref())
        .bind(query.reference_id.as_deref())
        .bind(query.level.as_deref())
        .bind(search)
        .fetch_one(&self.pool)
        .await?;
        let logs = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT to_jsonb(log_entries) FROM log_entries
                WHERE ($1::text IS NULL OR type::text = $1)
                  AND ($2::text IS NULL OR "referenceId" = $2)
                  AND ($3::text IS NULL OR level::text = $3)
                  AND ($4::text IS NULL OR data::text ILIKE '%' || $4 || '%')
                ORDER BY
                  CASE WHEN $5 = 'id' AND $6 = 'ASC' THEN id END ASC,
                  CASE WHEN $5 = 'id' AND $6 = 'DESC' THEN id END DESC,
                  CASE WHEN $5 = 'type' AND $6 = 'ASC' THEN type::text END ASC,
                  CASE WHEN $5 = 'type' AND $6 = 'DESC' THEN type::text END DESC,
                  CASE WHEN $5 = 'referenceId' AND $6 = 'ASC' THEN "referenceId" END ASC,
                  CASE WHEN $5 = 'referenceId' AND $6 = 'DESC' THEN "referenceId" END DESC,
                  CASE WHEN $5 = 'level' AND $6 = 'ASC' THEN level::text END ASC,
                  CASE WHEN $5 = 'level' AND $6 = 'DESC' THEN level::text END DESC,
                  CASE WHEN $5 = 'createdAt' AND $6 = 'ASC' THEN "createdAt" END ASC,
                  CASE WHEN $5 = 'createdAt' AND $6 = 'DESC' THEN "createdAt" END DESC,
                  CASE WHEN $5 = 'updatedAt' AND $6 = 'ASC' THEN "updatedAt" END ASC,
                  CASE WHEN $5 = 'updatedAt' AND $6 = 'DESC' THEN "updatedAt" END DESC,
                  CASE WHEN $5 = 'timestamp' AND $6 = 'ASC' THEN timestamp END ASC,
                  CASE WHEN $5 = 'timestamp' AND $6 = 'DESC' THEN timestamp END DESC
                LIMIT $7 OFFSET $8"#,
        )
        .bind(query.log_type.as_deref())
        .bind(query.reference_id.as_deref())
        .bind(query.level.as_deref())
        .bind(search)
        .bind(sort)
        .bind(order)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(serde_json::json!({
            "logs": logs,
            "total": total,
            "page": page,
            "pageSize": page_size
        }))
    }
}
