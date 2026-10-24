//! Persist faulty-build reports, alerts, and status transitions.
//!
//! Used by faulty-report HTTP handlers and `FaultyReportRepository`. Ports
//! Node `FaultyReportService` create/finalize/list/status flows, including
//! marking the related release `isFaulty` when a report is approved.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
    sea_query::Expr,
};
use uuid::Uuid;

use crate::persistence::entity::{alert, device, faulty_report, release, test_execution};

use super::{
    error::DeviceRepositoryError,
    postgres::json_models::{faulty_report_alert_json, faulty_report_json},
    repository_types::{FaultyReportCreate, FaultyReportCreation, FaultyReportFinalization},
};

/// PURPOSE: Insert a pending faulty report, resolving last test execution when omitted.
/// USE: Create-upload handler after validating the release exists; returns `None` if release missing.
pub(crate) async fn create(
    orm: &DatabaseConnection,
    request: &FaultyReportCreate,
) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_store", "create");
    let Ok(release_id) = Uuid::parse_str(&request.release_id) else {
        return Ok(None);
    };
    let release_exists = release::Entity::find_by_id(release_id).one(orm).await?.is_some();
    if !release_exists {
        return Ok(None);
    }

    // Prefer explicit test id; else latest execution for the build/family; else device lastTestExecution.
    let last_test_execution_id = match request.test_execution_id.as_deref() {
        Some(test_id) => Some(test_id.to_owned()),
        None => {
            let latest = test_execution::Entity::find()
                .filter(test_execution::Column::BuildId.eq(&request.release_id))
                .filter(test_execution::Column::DeviceFamily.eq(&request.device_family))
                .order_by_desc(test_execution::Column::CreatedAt)
                .select_only()
                .column(test_execution::Column::TestId)
                .into_tuple::<String>()
                .one(orm)
                .await?;
            match latest {
                Some(test_id) => Some(test_id),
                None => {
                    device::Entity::find()
                        .filter(device::Column::DeviceType.eq(&request.device_type))
                        .filter(device::Column::DeviceFamily.eq(&request.device_family))
                        .filter(device::Column::LastTestExecution.is_not_null())
                        .order_by_desc(device::Column::CreatedAt)
                        .select_only()
                        .column(device::Column::LastTestExecution)
                        .into_tuple::<Option<String>>()
                        .one(orm)
                        .await?
                        .flatten()
                }
            }
        }
    };

    let now = chrono::Utc::now();
    faulty_report::ActiveModel {
        id: Set(request.id),
        release_id: Set(release_id),
        device_type: Set(request.device_type.clone()),
        device_family: Set(request.device_family.clone()),
        last_test_execution_id: Set(last_test_execution_id.clone()),
        description: Set(request.description.clone()),
        file_path: Set(None),
        logs_path: Set(None),
        status: Set(faulty_report::FaultyStatus::Pending),
        created_by: Set(request.created_by.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(orm)
    .await?;

    Ok(Some(FaultyReportCreation {
        last_test_execution_id,
    }))
}

/// PURPOSE: Attach uploaded file/log paths and create an admin alert for the report.
/// USE: After multipart upload lands on disk; returns report + alert JSON for socket publish.
pub(crate) async fn finalize(
    orm: &DatabaseConnection,
    report_id: Uuid,
    file_path: Option<&str>,
    logs_path: Option<&str>,
) -> Result<FaultyReportFinalization, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::faulty_report_store", "finalize");
    let transaction = orm.begin().await?;
    let now = chrono::Utc::now();
    faulty_report::Entity::update_many()
        .col_expr(
            faulty_report::Column::FilePath,
            Expr::value(file_path.map(str::to_owned)),
        )
        .col_expr(
            faulty_report::Column::LogsPath,
            Expr::value(logs_path.map(str::to_owned)),
        )
        .col_expr(faulty_report::Column::UpdatedAt, Expr::value(now))
        .filter(faulty_report::Column::Id.eq(report_id))
        .exec(&transaction)
        .await?;

    let report_model = faulty_report::Entity::find_by_id(report_id)
        .one(&transaction)
        .await?
        .ok_or_else(|| DeviceRepositoryError::NotFound("Faulty report not found".to_owned()))?;
    let release_model = release::Entity::find_by_id(report_model.release_id)
        .one(&transaction)
        .await?
        .ok_or_else(|| DeviceRepositoryError::Internal("faulty report release missing".to_owned()))?;

    let data = serde_json::json!({
        "faultyReportId": report_model.id,
        "buildId": report_model.release_id,
        "buildVersion": release_model.version,
        "deviceType": report_model.device_type,
        "deviceFamily": report_model.device_family,
        "createdBy": report_model.created_by,
    });
    let alert_model = alert::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set("ADMIN".to_owned()),
        title: Set("Faulty Report".to_owned()),
        message: Set(format!(
            "New faulty report submitted for release {}",
            report_model.release_id
        )),
        data: Set(Some(data)),
        r#type: Set(alert::AlertType::Warning),
        status: Set(alert::AlertStatus::Unread),
        is_read: Set(false),
        read_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        build_id: Set(Some(report_model.release_id.to_string())),
        faulty_report_id: Set(Some(report_model.id.to_string())),
        device_id: Set(None),
        device_controller_id: Set(None),
        deleted_at: Set(None),
    }
    .insert(&transaction)
    .await?;

    let report = faulty_report_json(&report_model);
    let alert = faulty_report_alert_json(&alert_model, Some(&release_model.version));
    transaction.commit().await?;
    Ok(FaultyReportFinalization { report, alert })
}

/// PURPOSE: List all faulty reports newest-first as API JSON.
/// USE: Faulty report list handler.
pub(crate) async fn list(orm: &DatabaseConnection) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_store", "list");
    Ok(faulty_report::Entity::find()
        .order_by_desc(faulty_report::Column::CreatedAt)
        .all(orm)
        .await?
        .iter()
        .map(faulty_report_json)
        .collect())
}

/// PURPOSE: Fetch one report plus its release version.
/// USE: Detail / download handlers.
pub(crate) async fn by_id(
    orm: &DatabaseConnection,
    report_id: Uuid,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_store", "by_id");
    let Some((report, release_model)) = faulty_report::Entity::find_by_id(report_id)
        .find_also_related(release::Entity)
        .one(orm)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(serde_json::json!({
        "report": faulty_report_json(&report),
        "buildVersion": release_model
            .map(|r| r.version)
            .unwrap_or_else(|| "Unknown".to_owned()),
    })))
}

/// PURPOSE: Update report status; when approved, mark the release as faulty.
/// USE: Admin approve/reject actions.
pub(crate) async fn update_status(
    orm: &DatabaseConnection,
    report_id: Uuid,
    status: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::faulty_report_store", "update_status");
    let status_enum = match status {
        "pending" => faulty_report::FaultyStatus::Pending,
        "approved" => faulty_report::FaultyStatus::Approved,
        "rejected" => faulty_report::FaultyStatus::Rejected,
        _ => {
            return Err(DeviceRepositoryError::Validation(format!(
                "Invalid faulty report status: {status}"
            )));
        }
    };
    let transaction = orm.begin().await?;
    let now = chrono::Utc::now();
    let updated = faulty_report::Entity::update_many()
        .col_expr(faulty_report::Column::Status, Expr::value(status_enum))
        .col_expr(faulty_report::Column::UpdatedAt, Expr::value(now))
        .filter(faulty_report::Column::Id.eq(report_id))
        .exec(&transaction)
        .await?;
    if updated.rows_affected == 0 {
        transaction.commit().await?;
        return Ok(None);
    }
    // Approving a report flags the build so inventory / flashing refuse it (legacy FaultyReportService).
    if status == "approved" {
        let report = faulty_report::Entity::find_by_id(report_id)
            .one(&transaction)
            .await?
            .ok_or_else(|| DeviceRepositoryError::NotFound("Faulty report not found".to_owned()))?;
        release::Entity::update_many()
            .col_expr(release::Column::IsFaulty, Expr::value(Some(true)))
            .col_expr(release::Column::UpdatedAt, Expr::value(now))
            .filter(release::Column::Id.eq(report.release_id))
            .exec(&transaction)
            .await?;
    }
    let report = faulty_report::Entity::find_by_id(report_id)
        .one(&transaction)
        .await?
        .map(|row| faulty_report_json(&row));
    transaction.commit().await?;
    Ok(report)
}

/// PURPOSE: Hard-delete a faulty report row.
/// USE: Admin delete handler after optional file cleanup.
pub(crate) async fn delete(orm: &DatabaseConnection, report_id: Uuid) -> Result<bool, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_store", "delete");
    let result = faulty_report::Entity::delete_by_id(report_id).exec(orm).await?;
    Ok(result.rows_affected > 0)
}
