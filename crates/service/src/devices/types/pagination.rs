//! Shared pagination query helpers for device list endpoints.

use serde::Serialize;
use utoipa::ToSchema;

/// Request/response body type `PaginationResponse` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaginationResponse {
    /// 1-based page number for paginated lists.
    pub page: u32,
    /// Maximum rows to return (capped by MAX_PAGE_SIZE).
    pub limit: u32,
    /// `PaginationResponse.total_count` used by device HTTP handlers.
    pub total_count: i64,
    /// `PaginationResponse.total_pages` used by device HTTP handlers.
    pub total_pages: u64,
}
