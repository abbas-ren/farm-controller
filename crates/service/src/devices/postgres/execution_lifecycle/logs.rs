use super::*;

use crate::persistence::entity::log_entry;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
    sea_query::Expr,
};

impl PostgresDeviceRepository {
    pub(super) async fn create_log_inner(
        &self,
        request: &LogCreateRequest,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::logs", "create_log_inner");
        let log_type = parse_log_type(&request.log_type)?;
        let level = parse_log_level(&request.level)?;
        let now = chrono::Utc::now();
        let timestamp = request.timestamp.unwrap_or(now);
        let model = log_entry::ActiveModel {
            r#type: Set(log_type),
            reference_id: Set(request.reference_id.clone()),
            data: Set(request.data.clone()),
            level: Set(level),
            timestamp: Set(timestamp),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(self.orm())
        .await?;
        Ok(log_entry_json(&model))
    }

    pub(super) async fn list_logs_inner(
        &self,
        query: &LogListQuery,
        search: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError> {
        let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::logs", "list_logs_inner");
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
        let ascending = query.order.as_deref() == Some("ASC");

        let mut select = log_entry::Entity::find();
        if let Some(log_type) = query.log_type.as_deref() {
            select = select.filter(log_entry::Column::Type.eq(parse_log_type(log_type)?));
        }
        if let Some(reference_id) = query.reference_id.as_deref() {
            select = select.filter(log_entry::Column::ReferenceId.eq(reference_id));
        }
        if let Some(level) = query.level.as_deref() {
            select = select.filter(log_entry::Column::Level.eq(parse_log_level(level)?));
        }
        if let Some(search) = search {
            select = select.filter(
                Expr::cust_with_values("data::text ILIKE ('%' || $1 || '%')", [search]),
            );
        }

        let total = select.clone().count(self.orm()).await? as i64;

        let select = match (sort, ascending) {
            ("id", true) => select.order_by_asc(log_entry::Column::Id),
            ("id", false) => select.order_by_desc(log_entry::Column::Id),
            ("type", true) => select.order_by_asc(log_entry::Column::Type),
            ("type", false) => select.order_by_desc(log_entry::Column::Type),
            ("referenceId", true) => select.order_by_asc(log_entry::Column::ReferenceId),
            ("referenceId", false) => select.order_by_desc(log_entry::Column::ReferenceId),
            ("level", true) => select.order_by_asc(log_entry::Column::Level),
            ("level", false) => select.order_by_desc(log_entry::Column::Level),
            ("createdAt", true) => select.order_by_asc(log_entry::Column::CreatedAt),
            ("createdAt", false) => select.order_by_desc(log_entry::Column::CreatedAt),
            ("updatedAt", true) => select.order_by_asc(log_entry::Column::UpdatedAt),
            ("updatedAt", false) => select.order_by_desc(log_entry::Column::UpdatedAt),
            (_, true) => select.order_by_asc(log_entry::Column::Timestamp),
            (_, false) => select.order_by_desc(log_entry::Column::Timestamp),
        };

        let logs = select
            .limit(page_size as u64)
            .offset(offset as u64)
            .all(self.orm())
            .await?
            .iter()
            .map(log_entry_json)
            .collect::<Vec<_>>();

        Ok(serde_json::json!({
            "logs": logs,
            "total": total,
            "page": page,
            "pageSize": page_size
        }))
    }
}

fn parse_log_type(value: &str) -> Result<log_entry::LogEntryType, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::logs", "parse_log_type");
    match value {
        "build" => Ok(log_entry::LogEntryType::Build),
        "test" => Ok(log_entry::LogEntryType::Test),
        "device" => Ok(log_entry::LogEntryType::Device),
        "general" => Ok(log_entry::LogEntryType::General),
        _ => Err(DeviceRepositoryError::Validation(format!(
            "invalid log type: {value}"
        ))),
    }
}

fn parse_log_level(value: &str) -> Result<log_entry::LogEntryLevel, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::logs", "parse_log_level");
    match value {
        "info" => Ok(log_entry::LogEntryLevel::Info),
        "warn" => Ok(log_entry::LogEntryLevel::Warn),
        "error" => Ok(log_entry::LogEntryLevel::Error),
        "debug" => Ok(log_entry::LogEntryLevel::Debug),
        _ => Err(DeviceRepositoryError::Validation(format!(
            "invalid log level: {value}"
        ))),
    }
}

fn log_entry_json(model: &log_entry::Model) -> serde_json::Value {
    let _api_trace = crate::observability::ApiTrace::enter("devices::postgres::execution_lifecycle::logs", "log_entry_json");
    serde_json::json!({
        "id": model.id,
        "type": match model.r#type {
            log_entry::LogEntryType::Build => "build",
            log_entry::LogEntryType::Test => "test",
            log_entry::LogEntryType::Device => "device",
            log_entry::LogEntryType::General => "general",
        },
        "referenceId": model.reference_id,
        "data": model.data,
        "level": match model.level {
            log_entry::LogEntryLevel::Info => "info",
            log_entry::LogEntryLevel::Warn => "warn",
            log_entry::LogEntryLevel::Error => "error",
            log_entry::LogEntryLevel::Debug => "debug",
        },
        "timestamp": model.timestamp,
        "createdAt": model.created_at,
        "updatedAt": model.updated_at,
    })
}
