use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "retention_policies", schema_name = "farmcontroller")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub table_name: String,
    pub timestamp_column: String,
    pub retention_days: i32,
    pub batch_size: i32,
    pub enabled: bool,
    pub archive_before_delete: bool,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
