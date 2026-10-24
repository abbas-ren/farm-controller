use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_log_entries_type"
)]
pub enum LogEntryType {
    #[sea_orm(string_value = "build")]
    Build,
    #[sea_orm(string_value = "test")]
    Test,
    #[sea_orm(string_value = "device")]
    Device,
    #[sea_orm(string_value = "general")]
    General,
}
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_log_entries_level"
)]
pub enum LogEntryLevel {
    #[sea_orm(string_value = "info")]
    Info,
    #[sea_orm(string_value = "warn")]
    Warn,
    #[sea_orm(string_value = "error")]
    Error,
    #[sea_orm(string_value = "debug")]
    Debug,
}
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "log_entries", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub r#type: LogEntryType,
    pub reference_id: String,
    pub data: Json,
    pub level: LogEntryLevel,
    pub timestamp: DateTimeUtc,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
