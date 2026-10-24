//! IPL payload staging and SSH/SFTP distribution to device controllers.
//!
//! Prepares family-specific IPL trees, syncs them to controllers, and removes
//! obsolete remote builds when releases are deleted.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use russh::{client, keys::PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{
    config::DeviceConfig,
    persistence::entity::device_controller,
    state::AppState,
};

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, sea_query::Expr};

struct SshClient {
    host: String,
    port: u16,
    known_hosts_file: PathBuf,
    accept_unknown_host_keys: bool,
}

impl client::Handler for SshClient {
    type Error = russh::Error;

    /// SSH host-key callback for controller connections.
    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        if self.accept_unknown_host_keys {
            return Ok(true);
        }
        match russh::keys::check_known_hosts_path(
            &self.host,
            self.port,
            &server_public_key.public_key(),
            &self.known_hosts_file,
        ) {
            Ok(known) => Ok(known),
            Err(error) => {
                tracing::warn!(
                    host = self.host,
                    port = self.port,
                    %error,
                    "IPL SSH server host key verification failed"
                );
                Ok(false)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IplFamily {
    Gen4,
    Gen5,
}

impl IplFamily {
    /// Parse an IPL family token from a device-family string.
    fn parse(value: &str) -> Option<Self> {
        match value
            .chars()
            .filter(|character| !character.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect::<String>()
            .as_str()
        {
            "gen4" => Some(Self::Gen4),
            "gen5" => Some(Self::Gen5),
            _ => None,
        }
    }

    /// Canonical DB/token form for this IPL family.
    fn database_token(self) -> &'static str {
        match self {
            Self::Gen4 => "gen4",
            Self::Gen5 => "gen5",
        }
    }

    /// Filesystem path segment used for this family's IPL tree.
    fn storage_segment(self) -> &'static str {
        match self {
            Self::Gen4 => "Gen4",
            Self::Gen5 => "Gen5",
        }
    }
}

struct TransferTarget<'a> {
    username: &'a str,
    password: &'a str,
    port: u16,
    remote_path: &'a str,
}

/// Stage IPL payloads for a device family/version and upload to controllers.
pub(super) async fn prepare_and_distribute(
    state: &AppState,
    artifact_paths: &[PathBuf; 3],
    device_family: &str,
    version: &str,
) -> Result<(), String> {
    let Some(family) = IplFamily::parse(device_family) else {
        return Ok(());
    };
    let local_payload =
        stage_payload(artifact_paths, device_family, version, &state.config.device)?;
    let Some(target) = transfer_target(family, &state.config.device) else {
        tracing::warn!(
            device_family,
            version,
            "IPL payload stored locally; controller distribution credentials are not configured"
        );
        return Ok(());
    };
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable for IPL distribution".to_owned())?;
    let controllers = active_controllers(database.orm(), family).await?;
    if controllers.is_empty() {
        tracing::warn!(
            device_family,
            version,
            "no active controller accepts IPL payload"
        );
        return Ok(());
    }

    let timeout = Duration::from_secs(state.config.device.ipl_transfer_timeout_seconds);
    let mut failures = Vec::new();
    for host in controllers {
        let transfer = upload_to_controller(
            &host,
            &target,
            &local_payload,
            version,
            &state.config.device,
        );
        match tokio::time::timeout(timeout, transfer).await {
            Ok(Ok(())) => {
                tracing::info!(host, device_family, version, artifact_paths = ?artifact_paths, "distributed IPL payload");
            }
            Ok(Err(error)) => failures.push(format!("{host}: {error}")),
            Err(_) => failures.push(format!("{host}: transfer timed out")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "IPL distribution failed for {} controller(s): {}",
            failures.len(),
            failures.join(" | ")
        ))
    }
}

/// Resolve SSH target + remote root for a given IPL family.
fn transfer_target(family: IplFamily, config: &DeviceConfig) -> Option<TransferTarget<'_>> {
    let target = match family {
        IplFamily::Gen4 => TransferTarget {
            username: config.gen4_ipl_username.trim(),
            password: &config.gen4_ipl_password,
            port: config.gen4_ipl_port,
            remote_path: config.gen4_ipl_remote_path.trim(),
        },
        IplFamily::Gen5 => TransferTarget {
            username: config.gen5_ipl_username.trim(),
            password: &config.gen5_ipl_password,
            port: config.gen5_ipl_port,
            remote_path: config.gen5_ipl_remote_path.trim(),
        },
    };
    (!target.username.is_empty()).then_some(target)
}

/// Copy prepared IPL files into the local staging directory for upload.
fn stage_payload(
    artifact_paths: &[PathBuf; 3],
    device_family: &str,
    version: &str,
    config: &DeviceConfig,
) -> Result<PathBuf, String> {
    let source = artifact_paths[0]
        .parent()
        .ok_or_else(|| "release artifact directory is unavailable".to_owned())?
        .join("ipl");
    if !source.is_dir() {
        return Err(format!(
            "IPL payload directory is missing: {}",
            source.display()
        ));
    }
    let family = IplFamily::parse(device_family)
        .ok_or_else(|| format!("unsupported IPL device family: {device_family}"))?
        .storage_segment();
    let version = super::artifact_preparation::safe_segment(version)?;
    let parent = Path::new(&config.ipl_artifacts_dir).join(family);
    let destination = parent.join(version);
    fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
    let staging = parent.join(format!(".ipl-prep-{}", Uuid::new_v4()));
    fs::create_dir(&staging).map_err(|error| error.to_string())?;
    let result = (|| {
        copy_tree(&source, &staging)?;
        if IplFamily::parse(device_family) == Some(IplFamily::Gen5) {
            let script = Path::new(&config.gen5_ipl_script);
            if !script.is_file() {
                return Err(format!(
                    "required Gen5 IPL script is missing: {}",
                    script.display()
                ));
            }
            copy_file(script, &staging.join("gen5_ipl.sh"))?;
        }
        fs::write(staging.join(".prep-done"), chrono::Utc::now().to_rfc3339())
            .map_err(|error| error.to_string())?;
        if destination.exists() {
            fs::remove_dir_all(&destination).map_err(|error| error.to_string())?;
        }
        fs::rename(&staging, &destination).map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result.map(|()| destination)
}

/// Recursively copy a directory tree.
fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        if file_type.is_symlink() {
            return Err(format!(
                "symbolic links are not allowed in IPL payloads: {}",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            fs::create_dir(&target).map_err(|error| error.to_string())?;
            copy_tree(&entry.path(), &target)?;
        } else if file_type.is_file() {
            copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copy one file, creating parents as needed.
fn copy_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::copy(source, destination).map_err(|error| error.to_string())?;
    let permissions = fs::metadata(source)
        .map_err(|error| error.to_string())?
        .permissions();
    fs::set_permissions(destination, permissions).map_err(|error| error.to_string())
}

/// SFTP the staged payload to one controller.
async fn upload_to_controller(
    host: &str,
    target: &TransferTarget<'_>,
    local_payload: &Path,
    version: &str,
    config: &DeviceConfig,
) -> Result<(), String> {
    let sftp = open_sftp(host, target, config).await?;
    let remote_version = format!("{}/{}", target.remote_path.trim_end_matches('/'), version);
    ensure_remote_directory(&sftp, &remote_version).await?;

    for (local, relative, is_directory) in payload_entries(local_payload)? {
        let relative = relative
            .to_str()
            .ok_or_else(|| "IPL payload paths must be UTF-8".to_owned())?
            .replace('\\', "/");
        let remote = format!("{remote_version}/{relative}");
        if is_directory {
            ensure_remote_directory(&sftp, &remote).await?;
            continue;
        }
        let mut source = tokio::fs::File::open(&local)
            .await
            .map_err(|error| error.to_string())?;
        let mut destination = sftp
            .create(remote)
            .await
            .map_err(|error| error.to_string())?;
        tokio::io::copy(&mut source, &mut destination)
            .await
            .map_err(|error| error.to_string())?;
        destination
            .shutdown()
            .await
            .map_err(|error| error.to_string())?;
    }
    sftp.close().await.map_err(|error| error.to_string())
}

/// Open an SSH+SFTP session with the configured credentials.
async fn open_sftp(
    host: &str,
    target: &TransferTarget<'_>,
    config: &DeviceConfig,
) -> Result<SftpSession, String> {
    let ssh_client = SshClient {
        host: host.to_owned(),
        port: target.port,
        known_hosts_file: config.terminal_known_hosts_file.clone().into(),
        accept_unknown_host_keys: config.terminal_accept_unknown_host_keys,
    };
    let mut ssh = client::connect(
        Arc::new(client::Config::default()),
        (host, target.port),
        ssh_client,
    )
    .await
    .map_err(|error| error.to_string())?;
    if !ssh
        .authenticate_password(target.username, target.password)
        .await
        .map_err(|error| error.to_string())?
        .success()
    {
        return Err("authentication failed".to_owned());
    }
    let channel = ssh
        .channel_open_session()
        .await
        .map_err(|error| error.to_string())?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| error.to_string())?;
    SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| error.to_string())
}

/// Re-sync all local IPL versions to one controller (registration hook).
pub(crate) async fn sync_controller(
    state: &AppState,
    host: &str,
    device_family: &str,
) -> Result<(), String> {
    let Some(family) = IplFamily::parse(device_family) else {
        return Ok(());
    };
    let Some(target) = transfer_target(family, &state.config.device) else {
        return Ok(());
    };
    let family_dir =
        Path::new(&state.config.device.ipl_artifacts_dir).join(family.storage_segment());
    let versions = version_directories(&family_dir)?;
    let timeout = Duration::from_secs(state.config.device.ipl_transfer_timeout_seconds);
    let mut failures = Vec::new();
    for (version, payload) in versions {
        match tokio::time::timeout(
            timeout,
            upload_to_controller(host, &target, &payload, &version, &state.config.device),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(format!("{version}: {error}")),
            Err(_) => failures.push(format!("{version}: transfer timed out")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "failed to synchronize IPL payloads to {host}: {}",
            failures.join(" | ")
        ))
    }
}

/// Delete a version tree remotely (and optionally locally) across controllers.
pub(crate) async fn remove_build(
    state: &AppState,
    device_family: &str,
    version: &str,
) -> Result<(), String> {
    let Some(family) = IplFamily::parse(device_family) else {
        return Ok(());
    };
    let version = super::artifact_preparation::safe_segment(version)?.to_owned();
    let local_path = Path::new(&state.config.device.ipl_artifacts_dir)
        .join(family.storage_segment())
        .join(&version);
    if let Err(error) = tokio::fs::remove_dir_all(&local_path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(error.to_string());
    }
    let Some(target) = transfer_target(family, &state.config.device) else {
        return Ok(());
    };
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable for IPL cleanup".to_owned())?;
    let controllers = active_controllers(database.orm(), family).await?;
    let remote_path = format!("{}/{}", target.remote_path.trim_end_matches('/'), version);
    let timeout = Duration::from_secs(state.config.device.ipl_transfer_timeout_seconds);
    let mut failures = Vec::new();
    for host in controllers {
        let cleanup = async {
            let sftp = open_sftp(&host, &target, &state.config.device).await?;
            remove_remote_tree(&sftp, &remote_path).await?;
            sftp.close().await.map_err(|error| error.to_string())
        };
        match tokio::time::timeout(timeout, cleanup).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(format!("{host}: {error}")),
            Err(_) => failures.push(format!("{host}: cleanup timed out")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "IPL cleanup failed for {} controller(s): {}",
            failures.len(),
            failures.join(" | ")
        ))
    }
}

/// List controllers that should receive IPL sync for a family.
async fn active_controllers(
    orm: &sea_orm::DatabaseConnection,
    family: IplFamily,
) -> Result<Vec<String>, String> {
    device_controller::Entity::find()
        .filter(Expr::cust_with_values(
            "replace(lower(\"deviceFamily\"), ' ', '') = $1",
            [family.database_token()],
        ))
        .filter(
            device_controller::Column::Status.eq(device_controller::DeviceControllerStatus::Approved),
        )
        .filter(
            device_controller::Column::State.eq(device_controller::DeviceControllerState::Active),
        )
        .filter(device_controller::Column::DeletedAt.is_null())
        .order_by_asc(device_controller::Column::IpAddress)
        .select_only()
        .column(device_controller::Column::IpAddress)
        .distinct()
        .into_tuple::<String>()
        .all(orm)
        .await
        .map_err(|error| error.to_string())
}

/// Recursively remove a remote SFTP directory.
async fn remove_remote_tree(sftp: &SftpSession, path: &str) -> Result<(), String> {
    validate_remote_path(path)?;
    if !sftp
        .try_exists(path.to_owned())
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(());
    }
    let mut pending = vec![path.to_owned()];
    let mut directories = Vec::new();
    while let Some(directory) = pending.pop() {
        directories.push(directory.clone());
        for entry in sftp
            .read_dir(directory)
            .await
            .map_err(|error| error.to_string())?
        {
            if entry.file_type().is_dir() {
                pending.push(entry.path());
            } else {
                sftp.remove_file(entry.path())
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    directories.sort_by_key(|directory| std::cmp::Reverse(directory.matches('/').count()));
    for directory in directories {
        sftp.remove_dir(directory)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Enumerate local version directories under an IPL root.
fn version_directories(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut versions = fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_dir()))
        .filter_map(|entry| {
            entry
                .file_name()
                .into_string()
                .ok()
                .map(|version| (version, entry.path()))
        })
        .filter(|(version, _)| !version.starts_with('.'))
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(versions)
}

/// mkdir -p equivalent over SFTP.
async fn ensure_remote_directory(sftp: &SftpSession, path: &str) -> Result<(), String> {
    validate_remote_path(path)?;
    let mut current = String::new();
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        current.push('/');
        current.push_str(segment);
        if !sftp
            .try_exists(current.clone())
            .await
            .map_err(|error| error.to_string())?
        {
            sftp.create_dir(current.clone())
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// Reject empty or traversal-prone remote paths.
fn validate_remote_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err(format!("remote IPL path must be absolute: {path}"));
    }
    if path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .any(|segment| matches!(segment, "." | ".."))
    {
        return Err(format!("unsafe remote IPL path: {path}"));
    }
    Ok(())
}

/// Walk a staged payload into (relative, absolute, is_dir) entries.
fn payload_entries(root: &Path) -> Result<Vec<(PathBuf, PathBuf, bool)>, String> {
    /// Directory walk visitor collecting payload file entries.
    fn visit(
        root: &Path,
        directory: &Path,
        entries: &mut Vec<(PathBuf, PathBuf, bool)>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                return Err(format!(
                    "symbolic links are not allowed in IPL payloads: {}",
                    entry.path().display()
                ));
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_path_buf();
            entries.push((entry.path(), relative, file_type.is_dir()));
            if file_type.is_dir() {
                visit(root, &entry.path(), entries)?;
            } else if !file_type.is_file() {
                return Err(format!(
                    "unsupported IPL payload entry: {}",
                    entry.path().display()
                ));
            }
        }
        Ok(())
    }

    let mut entries = Vec::new();
    visit(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper returning the three standard IPL artifact paths.
    fn artifacts(root: &Path) -> [PathBuf; 3] {
        [
            root.join("Image"),
            root.join("board.dtb"),
            root.join("root.tar.bz2"),
        ]
    }

    #[test]
    /// Unit test: Gen4 staging layout by family/version.
    fn stages_gen4_payload_by_family_and_version() {
        let directory = tempfile::tempdir().unwrap();
        let release = directory.path().join("release");
        fs::create_dir_all(release.join("ipl")).unwrap();
        fs::write(release.join("ipl/boot.bin"), b"boot").unwrap();
        let config = DeviceConfig {
            ipl_artifacts_dir: directory
                .path()
                .join("stored")
                .to_string_lossy()
                .into_owned(),
            ..DeviceConfig::default()
        };

        let stored = stage_payload(&artifacts(&release), "Gen4", "v1.2.3", &config).unwrap();

        assert_eq!(fs::read(stored.join("boot.bin")).unwrap(), b"boot");
        assert!(stored.join(".prep-done").is_file());
        assert!(stored.ends_with("Gen4/v1.2.3"));
    }

    #[test]
    /// Unit test: Gen5 script + nested payload staging.
    fn stages_gen5_script_and_nested_payload() {
        let directory = tempfile::tempdir().unwrap();
        let release = directory.path().join("release");
        fs::create_dir_all(release.join("ipl/hil")).unwrap();
        fs::write(release.join("ipl/boot.tar.gz"), b"boot").unwrap();
        fs::write(release.join("ipl/hil/config"), b"hil").unwrap();
        let script = directory.path().join("gen5_ipl.sh");
        fs::write(&script, b"script").unwrap();
        let config = DeviceConfig {
            ipl_artifacts_dir: directory
                .path()
                .join("stored")
                .to_string_lossy()
                .into_owned(),
            gen5_ipl_script: script.to_string_lossy().into_owned(),
            ..DeviceConfig::default()
        };

        let stored = stage_payload(&artifacts(&release), "Gen 5", "v2", &config).unwrap();

        assert_eq!(fs::read(stored.join("hil/config")).unwrap(), b"hil");
        assert_eq!(fs::read(stored.join("gen5_ipl.sh")).unwrap(), b"script");
    }

    #[test]
    /// Unit test: family normalize + remote path validation.
    fn normalizes_families_and_rejects_unsafe_remote_paths() {
        assert!(IplFamily::parse("Gen 4") == Some(IplFamily::Gen4));
        assert!(IplFamily::parse("other").is_none());
        assert!(validate_remote_path("/home/racer/RACER_IPL").is_ok());
        assert!(validate_remote_path("relative/path").is_err());
        assert!(validate_remote_path("/home/../root").is_err());
    }

    #[test]
    /// Unit test: incomplete version dirs are skipped.
    fn enumerates_only_complete_version_directories() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("v2")).unwrap();
        fs::create_dir(directory.path().join("v1")).unwrap();
        fs::create_dir(directory.path().join(".ipl-prep-incomplete")).unwrap();
        fs::write(directory.path().join("not-a-version"), b"file").unwrap();

        let versions = version_directories(directory.path()).unwrap();

        assert_eq!(
            versions
                .into_iter()
                .map(|(version, _)| version)
                .collect::<Vec<_>>(),
            ["v1", "v2"]
        );
    }
}
