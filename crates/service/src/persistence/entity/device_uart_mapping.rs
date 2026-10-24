use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_uart_mappings", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub device_id: String,
    pub controller_id: String,
    pub relay_channel_id: Option<Uuid>,
    pub device_mac: String,
    pub generation: i16,
    pub requested_vid_pid: String,
    pub tty: String,
    pub usb_serial: Option<String>,
    pub interface: i16,
    pub topology: String,
    pub connection: String,
    pub verified: bool,
    pub verified_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
