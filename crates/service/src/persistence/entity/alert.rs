//! User-facing alert rows (`alerts` table).
//!
//! Ports `farm_controller_old/device-service/src/models/Alert.ts`. Alerts notify
//! operators about builds, faulty reports, devices, and controllers. Retention
//! cleanup deletes only `read` alerts older than the configured window.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Severity / presentation category for an alert.
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "enum_alerts_type")]
pub enum AlertType {
    /// Informational notice.
    #[sea_orm(string_value = "info")]
    Info,
    /// Successful completion notice.
    #[sea_orm(string_value = "success")]
    Success,
    /// Non-fatal warning.
    #[sea_orm(string_value = "warning")]
    Warning,
    /// Failure or blocking condition.
    #[sea_orm(string_value = "error")]
    Error,
}

/// Read/unread inbox state. Retention only purges [`AlertStatus::Read`].
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "enum_alerts_status")]
pub enum AlertStatus {
    /// Still visible as unread in the UI.
    #[sea_orm(string_value = "unread")]
    Unread,
    /// Acknowledged; eligible for retention deletion.
    #[sea_orm(string_value = "read")]
    Read,
}

/// One alert delivered to a user inbox.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "alerts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Recipient user identifier (Keycloak / auth subject).
    pub user_id: String,
    pub title: String,
    pub message: String,
    /// Optional structured payload for deep links in the UI.
    pub data: Option<Json>,
    pub r#type: AlertType,
    pub status: AlertStatus,
    pub is_read: bool,
    pub read_at: Option<DateTimeUtc>,
    /// Legacy snake_case column; retention filters on this timestamp.
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub build_id: Option<String>,
    pub faulty_report_id: Option<String>,
    pub device_id: Option<String>,
    pub device_controller_id: Option<String>,
    pub deleted_at: Option<DateTimeUtc>,
}

/// No SeaORM relations declared; foreign keys are soft references only.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// PURPOSE: Expose the denormalized read flag for API serializers.
    /// USE: Call when mapping alerts to responses that need a boolean `isRead`.
    pub fn is_read(&self) -> bool {
        self.is_read
    }
}
