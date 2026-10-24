//! Verified UART / USB serial mappings (`device_uart_mappings` table).
//!
//! Captures the durable binding between a DUT, its controller, optional relay
//! channel, and tty path after Gen5 (or similar) mapping verification. Used by
//! flash/test workers to open the correct serial console.

use sea_orm::entity::prelude::*;

/// One verified serial-console mapping for a device.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "device_uart_mappings", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub device_id: String,
    pub controller_id: String,
    /// Optional power-relay channel associated with this mapping.
    pub relay_channel_id: Option<Uuid>,
    pub device_mac: String,
    /// Hardware generation (e.g. Gen5) used by mapping workers.
    pub generation: i16,
    /// USB VID:PID the mapping was requested for.
    pub requested_vid_pid: String,
    /// Host tty path (e.g. `/dev/ttyUSB0`).
    pub tty: String,
    pub usb_serial: Option<String>,
    pub interface: i16,
    /// USB topology string from the controller agent.
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
