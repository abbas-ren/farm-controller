use crate::socket::types::farm::ServerEvent;

#[derive(Debug, sea_orm::FromQueryResult)]
pub struct StaleAction {
    pub id: uuid::Uuid,
    pub job_type: String,
    pub test_id: Option<String>,
    pub release_id: Option<uuid::Uuid>,
    pub device_id: Option<String>,
    pub device_type: String,
    pub user_id: String,
    pub build_id: Option<String>,
    pub created_by: Option<String>,
    pub version: Option<String>,
}

pub struct ActionTransition {
    pub events: Vec<ServerEvent>,
}
