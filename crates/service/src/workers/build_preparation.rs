use std::path::PathBuf;

use uuid::Uuid;

use crate::state::AppState;

use super::artifact_preparation::{artifact_paths, prepare_artifacts, safe_segment};

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
    sqlx::query(
        r#"INSERT INTO device_action_queue
              (id, "deviceType", "userId", type, status, mode, "releaseId", version,
               "createdAt", "updatedAt")
           SELECT $1, $2, $3, 'flash', 'queued', 'deviceType', $4, $5, now(), now()
           WHERE NOT EXISTS (
               SELECT 1 FROM device_action_queue
               WHERE "releaseId" = $4 AND type::text = 'flash'
                 AND status::text IN ('queued', 'running', 'completed'))"#,
    )
    .bind(Uuid::new_v4())
    .bind(device_type)
    .bind(user_id)
    .bind(release_id)
    .bind(version)
    .execute(database.pool())
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
