use serde::Serialize;
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaginationResponse {
    pub page: u32,
    pub limit: u32,
    pub total_count: i64,
    pub total_pages: u64,
}
