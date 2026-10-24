use std::path::PathBuf;

use uuid::Uuid;

use crate::{
    persistence::entity::device_action_queue,
    state::AppState,
};

use super::artifact_preparation::{artifact_paths, prepare_artifacts, safe_segment};

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

pub(crate) async fn prepare_uploaded_build(
    state: &AppState,
    release: &serde_json::Value,
    user_id: &str,
) -> Result<(), String> {
    if state.database.is_none() {
        return Ok(());
    }
    let release_id = release_string(release, "id")?;
    let release_id = Uuid::parse_str(release_id).map_err(|error| error.to_string())?;
    let version = safe_segment(release_string(release, "version")?)?.to_owned();
    let device_type = safe_segment(release_string(release, "deviceType")?)?.to_owned();
    let device_family = release_string(release, "deviceFamily")?.to_owned();
    let paths = artifact_paths(
        release
            .get("artifacts")
            .ok_or_else(|| "release artifacts are missing".to_owned())?,
        &state.config.device.rootfs_archive_extension,
    )?;
    let nfs_path = PathBuf::from(&state.config.device.nfs_host_path)
        .join(&device_type)
        .join(&version);
    let tftp_path = PathBuf::from(&state.config.device.tftp_output_dir)
        .join(&device_type)
        .join(&version);
    let preparation_paths = paths.clone();
    tokio::task::spawn_blocking(move || {
        prepare_artifacts(&preparation_paths, &nfs_path, &tftp_path)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;

    if release_string(release, "buildType")? == "official" {
        enqueue_official_build(state, release_id, &device_type, &version, user_id).await?;
    }
    super::ipl_distribution::prepare_and_distribute(state, &paths, &device_family, &version).await
}

pub(super) async fn enqueue_official_build(
    state: &AppState,
    release_id: Uuid,
    device_type: &str,
    version: &str,
    user_id: &str,
) -> Result<(), String> {
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable".to_owned())?;
    let existing = device_action_queue::Entity::find()
        .filter(device_action_queue::Column::ReleaseId.eq(release_id))
        .filter(device_action_queue::Column::Type.eq(device_action_queue::QueueJobType::Flash))
        .filter(device_action_queue::Column::Status.is_in([
            device_action_queue::QueueJobStatus::Queued,
            device_action_queue::QueueJobStatus::Running,
            device_action_queue::QueueJobStatus::Completed,
        ]))
        .one(database.orm())
        .await
        .map_err(|error| error.to_string())?;
    if existing.is_some() {
        return Ok(());
    }
    let now = chrono::Utc::now();
    device_action_queue::ActiveModel {
        id: Set(Uuid::new_v4()),
        device_type: Set(device_type.to_owned()),
        user_id: Set(user_id.to_owned()),
        r#type: Set(device_action_queue::QueueJobType::Flash),
        status: Set(device_action_queue::QueueJobStatus::Queued),
        mode: Set(device_action_queue::QueueDispatchMode::DeviceType),
        release_id: Set(Some(release_id)),
        version: Set(Some(version.to_owned())),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(database.orm())
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn release_string<'a>(release: &'a serde_json::Value, field: &str) -> Result<&'a str, String> {
    release
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("release {field} is missing"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_nonempty_release_fields() {
        let release = serde_json::json!({"version": "v1.2.3"});
        assert_eq!(
            release_string(&release, "deviceType").unwrap_err(),
            "release deviceType is missing"
        );
    }
}
