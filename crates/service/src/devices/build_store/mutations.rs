use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, DatabaseConnection, EntityTrait, Set, TransactionTrait,
};
use uuid::Uuid;

use crate::persistence::entity::{alert, release};

use super::super::{
    error::DeviceRepositoryError,
    repository_types::{BuildDeleteTarget, BuildFlagResult, BuildStatusResult},
};
use super::uploads::alert_json;

pub(crate) async fn update_build_status(
    orm: &DatabaseConnection,
    build_id: &str,
    status: &str,
) -> Result<Option<BuildStatusResult>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter(
        "devices::build_store::mutations",
        "update_build_status",
    );
    let Ok(build_uuid) = Uuid::parse_str(build_id) else {
        return Ok(None);
    };
    let status_enum = parse_release_status(status)?;
    let is_faulty = status == "failed";
    let transaction = orm.begin().await?;
    let Some(existing) = release::Entity::find_by_id(build_uuid)
        .one(&transaction)
        .await?
    else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let now = Utc::now();
    let updated = release::ActiveModel {
        id: Set(existing.id),
        status: Set(Some(status_enum)),
        is_faulty: Set(Some(is_faulty)),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(&transaction)
    .await?;
    let version = updated.version.clone();
    let device_type = updated.device_type.clone();
    let alert_model = alert::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set("ADMIN".to_owned()),
        title: Set("Build Status Updated".to_owned()),
        message: Set(format!("Build {version} status changed to {status}")),
        r#type: Set(alert::AlertType::Info),
        status: Set(alert::AlertStatus::Unread),
        is_read: Set(false),
        build_id: Set(Some(build_id.to_owned())),
        data: Set(Some(serde_json::json!({
            "buildId": build_id,
            "version": version,
            "deviceType": device_type,
            "status": status,
            "isFaulty": is_faulty,
        }))),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&transaction)
    .await?;
    transaction.commit().await?;
    Ok(Some(BuildStatusResult {
        version,
        device_type,
        is_faulty,
        alert: alert_json(&alert_model),
    }))
}

pub(crate) async fn flag_build(
    orm: &DatabaseConnection,
    build_id: &str,
    is_faulty: bool,
) -> Result<Option<BuildFlagResult>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::build_store::mutations", "flag_build");
    let Ok(build_uuid) = Uuid::parse_str(build_id) else {
        return Ok(None);
    };
    let transaction = orm.begin().await?;
    let Some(existing) = release::Entity::find_by_id(build_uuid)
        .one(&transaction)
        .await?
    else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let now = Utc::now();
    let status = if is_faulty {
        release::ReleaseTestStatus::Failed
    } else {
        release::ReleaseTestStatus::Passed
    };
    let updated = release::ActiveModel {
        id: Set(existing.id),
        is_faulty: Set(Some(is_faulty)),
        status: Set(Some(status)),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(&transaction)
    .await?;
    let version = updated.version.clone();
    let device_type = updated.device_type.clone();
    let alert_type = if is_faulty {
        alert::AlertType::Warning
    } else {
        alert::AlertType::Success
    };
    let state = if is_faulty {
        "flagged as faulty"
    } else {
        "marked as healthy"
    };
    let alert_model = alert::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set("ADMIN".to_owned()),
        title: Set("Build Flagged".to_owned()),
        message: Set(format!("Build {version} {state}")),
        r#type: Set(alert_type),
        status: Set(alert::AlertStatus::Unread),
        is_read: Set(false),
        build_id: Set(Some(build_id.to_owned())),
        data: Set(Some(serde_json::json!({
            "buildId": build_id,
            "version": version,
            "deviceType": device_type,
            "isFaulty": is_faulty,
        }))),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&transaction)
    .await?;
    transaction.commit().await?;
    Ok(Some(BuildFlagResult {
        version,
        device_type,
        alert: alert_json(&alert_model),
    }))
}

pub(crate) async fn delete_build(
    orm: &DatabaseConnection,
    build_id: &str,
) -> Result<Option<BuildDeleteTarget>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::build_store::mutations", "delete_build");
    let Ok(build_uuid) = Uuid::parse_str(build_id) else {
        return Ok(None);
    };
    let transaction = orm.begin().await?;
    let Some(existing) = release::Entity::find_by_id(build_uuid)
        .one(&transaction)
        .await?
    else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let folder_name = existing.folder_name.clone();
    let version = existing.version.clone();
    let device_family = existing.device_family.clone();
    let device_type = existing.device_type.clone();
    release::Entity::delete_by_id(build_uuid)
        .exec(&transaction)
        .await?;
    let now = Utc::now();
    let alert_model = alert::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set("ADMIN".to_owned()),
        title: Set("Build Deleted".to_owned()),
        message: Set(format!("Build {version} deleted")),
        r#type: Set(alert::AlertType::Error),
        status: Set(alert::AlertStatus::Unread),
        is_read: Set(false),
        build_id: Set(Some(build_id.to_owned())),
        data: Set(Some(serde_json::json!({
            "buildId": build_id,
            "version": version,
            "deviceType": device_type,
        }))),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&transaction)
    .await?;
    transaction.commit().await?;
    Ok(Some(BuildDeleteTarget {
        folder_name,
        version,
        device_family,
        device_type,
        alert: alert_json(&alert_model),
    }))
}

fn parse_release_status(status: &str) -> Result<release::ReleaseTestStatus, DeviceRepositoryError> {
    match status {
        "untested" => Ok(release::ReleaseTestStatus::Untested),
        "testing" => Ok(release::ReleaseTestStatus::Testing),
        "passed" => Ok(release::ReleaseTestStatus::Passed),
        "failed" => Ok(release::ReleaseTestStatus::Failed),
        _ => Err(DeviceRepositoryError::Internal(format!(
            "Invalid release status: {status}"
        ))),
    }
}
