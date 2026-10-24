use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "enum_alerts_type")]
pub enum AlertType {
    #[sea_orm(string_value = "info")]
    Info,
    #[sea_orm(string_value = "success")]
    Success,
    #[sea_orm(string_value = "warning")]
    Warning,
    #[sea_orm(string_value = "error")]
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "enum_alerts_status")]
pub enum AlertStatus {
    #[sea_orm(string_value = "unread")]
    Unread,
    #[sea_orm(string_value = "read")]
    Read,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "alerts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: String,
    pub title: String,
    pub message: String,
    pub data: Option<Json>,
    pub r#type: AlertType,
    pub status: AlertStatus,
    pub is_read: bool,
    pub read_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub build_id: Option<String>,
    pub faulty_report_id: Option<String>,
    pub device_id: Option<String>,
    pub device_controller_id: Option<String>,
    pub deleted_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

impl Model {
    pub fn is_read(&self) -> bool {
        self.is_read
    }
}
