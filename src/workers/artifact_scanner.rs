use std::{collections::HashSet, path::PathBuf, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use reqwest::{Client, Url, header};
use scraper::{Html, Selector};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{config::ArtifactCleanup, events::ServerEvent, state::AppState};

use super::{
    artifact_preparation::{prepare_artifacts, safe_segment},
    build_preparation::enqueue_official_build,
};

#[derive(Clone, Debug, Serialize)]
struct ArtifactInfo {
    name: String,
    size: u64,
    mtime: DateTime<Utc>,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(skip)]
    source_url: Url,
}

#[derive(Debug)]
struct ScanResult {
    folder_name: String,
    version: String,
    artifacts: Vec<ArtifactInfo>,
}

struct Scanner {
    root: Url,
    client: Client,
    compute_sha256: bool,
}

pub(super) async fn worker(state: Arc<AppState>, cancellation: CancellationToken) {
    let base_url = state.config.device.artifacts_base_url.trim();
    if base_url.is_empty() {
        tracing::info!(
            worker = "artifact_scanner",
            "artifact scanner disabled without a base URL"
        );
        return;
    }
    let scanner = match Scanner::new(
        base_url,
        state.config.workers.artifact_scan_request_timeout_seconds,
        state.config.workers.artifact_scan_compute_sha256,
    ) {
        Ok(scanner) => scanner,
        Err(error) => {
            tracing::error!(%error, worker = "artifact_scanner", "invalid artifact scanner configuration");
            return;
        }
    };
    let mut interval = tokio::time::interval(Duration::from_secs(
        state.config.workers.artifact_scan_poll_seconds,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => run_cycle(&state, &scanner).await,
        }
    }
    tracing::info!(worker = "artifact_scanner", "worker stopped");
}

async fn run_cycle(state: &AppState, scanner: &Scanner) {
    match scanner.scan_once().await {
        Ok(results) => {
            let count = results.len() as u64;
            match persist_results(state, scanner, results).await {
                Ok(()) => {
                    state.metrics.record_worker_items("artifact_scanner", count);
                    state
                        .metrics
                        .record_worker_run("artifact_scanner", "success");
                }
                Err(error) => {
                    tracing::error!(%error, worker = "artifact_scanner", "failed to persist artifact scan");
                    state
                        .metrics
                        .record_worker_run("artifact_scanner", "failure");
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, worker = "artifact_scanner", "artifact scan failed");
            state
                .metrics
                .record_worker_run("artifact_scanner", "failure");
        }
    }
}

impl Scanner {
    fn new(base_url: &str, timeout_seconds: u64, compute_sha256: bool) -> Result<Self, String> {
        let mut root = Url::parse(base_url).map_err(|error| error.to_string())?;
        if !root.path().ends_with('/') {
            root.set_path(&format!("{}/", root.path()));
        }
        let client = crate::external_http::client(Duration::from_secs(timeout_seconds))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            root,
            client,
            compute_sha256,
        })
    }

    async fn scan_once(&self) -> Result<Vec<ScanResult>, String> {
        let mut results = Vec::new();
        for folder_href in self.directory_links(&self.root).await? {
            let folder_url = self.scoped_url(&self.root, &folder_href)?;
            let folder_name = final_path_segment(&folder_url)?;
            for version_href in self.directory_links(&folder_url).await? {
                let version_url = self.scoped_url(&folder_url, &version_href)?;
                let version = final_path_segment(&version_url)?;
                let hrefs = self.index_links(&version_url).await?;
                let checksums = self.sidecar_checksums(&version_url, &hrefs).await;
                let mut artifacts = Vec::new();
                let mut ipl_directories = Vec::new();
                for href in hrefs {
                    let source_url = self.scoped_url(&version_url, &href)?;
                    if href.ends_with('/') {
                        if final_path_segment(&source_url)?.eq_ignore_ascii_case("ipl") {
                            ipl_directories.push(source_url);
                        }
                        continue;
                    }
                    let Some((size, mtime)) = self.file_metadata(&source_url).await else {
                        continue;
                    };
                    let name = final_path_segment(&source_url)?;
                    let sha256 = match checksums
                        .iter()
                        .find(|(filename, _)| filename == &name)
                        .map(|(_, hash)| hash.clone())
                    {
                        Some(hash) => Some(hash),
                        None if self.compute_sha256 => self.file_sha256(&source_url).await,
                        None => None,
                    };
                    artifacts.push(ArtifactInfo {
                        name,
                        size,
                        mtime,
                        path: self.relative_path(&source_url),
                        sha256,
                        source_url,
                    });
                }
                for directory in ipl_directories {
                    artifacts.extend(
                        self.collect_nested_artifacts(directory, "ipl/".to_owned())
                            .await?,
                    );
                }
                results.push(ScanResult {
                    folder_name: folder_name.clone(),
                    version,
                    artifacts,
                });
            }
        }
        Ok(results)
    }

    async fn directory_links(&self, url: &Url) -> Result<Vec<String>, String> {
        Ok(self
            .index_links(url)
            .await?
            .into_iter()
            .filter(|href| href.ends_with('/'))
            .collect())
    }

    async fn collect_nested_artifacts(
        &self,
        directory: Url,
        prefix: String,
    ) -> Result<Vec<ArtifactInfo>, String> {
        let mut pending = vec![(directory, prefix)];
        let mut visited = HashSet::new();
        let mut artifacts = Vec::new();
        while let Some((directory, prefix)) = pending.pop() {
            if !visited.insert(directory.clone()) {
                continue;
            }
            let hrefs = self.index_links(&directory).await?;
            let checksums = self.sidecar_checksums(&directory, &hrefs).await;
            for href in hrefs {
                let source_url = self.scoped_url(&directory, &href)?;
                let base_name = final_path_segment(&source_url)?;
                if href.ends_with('/') {
                    pending.push((source_url, format!("{prefix}{base_name}/")));
                    continue;
                }
                let Some((size, mtime)) = self.file_metadata(&source_url).await else {
                    continue;
                };
                let sha256 = match checksums
                    .iter()
                    .find(|(filename, _)| filename == &base_name)
                    .map(|(_, hash)| hash.clone())
                {
                    Some(hash) => Some(hash),
                    None if self.compute_sha256 => self.file_sha256(&source_url).await,
                    None => None,
                };
                artifacts.push(ArtifactInfo {
                    name: format!("{prefix}{base_name}"),
                    size,
                    mtime,
                    path: self.relative_path(&source_url),
                    sha256,
                    source_url,
                });
            }
        }
        Ok(artifacts)
    }

    async fn index_links(&self, url: &Url) -> Result<Vec<String>, String> {
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|error| error.to_string())?
            .error_for_status()
            .map_err(|error| error.to_string())?;
        let document =
            Html::parse_document(&response.text().await.map_err(|error| error.to_string())?);
        let selector = Selector::parse("a[href]").map_err(|error| error.to_string())?;
        Ok(document
            .select(&selector)
            .filter_map(|link| link.value().attr("href"))
            .filter(|href| !matches!(*href, ".." | "../") && !href.starts_with(['#', '?']))
            .map(str::to_owned)
            .collect())
    }

    fn scoped_url(&self, base: &Url, href: &str) -> Result<Url, String> {
        let url = base.join(href).map_err(|error| error.to_string())?;
        if url.origin() != self.root.origin() || !url.path().starts_with(self.root.path()) {
            return Err(format!("artifact link escapes configured root: {href}"));
        }
        Ok(url)
    }

    async fn file_metadata(&self, url: &Url) -> Option<(u64, DateTime<Utc>)> {
        let response = match self.client.head(url.clone()).send().await {
            Ok(response) if response.status().is_success() => response,
            _ => self
                .client
                .get(url.clone())
                .header(header::RANGE, "bytes=0-0")
                .send()
                .await
                .ok()?,
        };
        if !response.status().is_success() {
            return None;
        }
        let size = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit_once('/'))
            .and_then(|(_, value)| value.parse().ok())
            .or_else(|| {
                response
                    .headers()
                    .get(header::CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or(0);
        let mtime = response
            .headers()
            .get(header::LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| DateTime::parse_from_rfc2822(value).ok())
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        Some((size, mtime))
    }

    async fn sidecar_checksums(&self, base: &Url, hrefs: &[String]) -> Vec<(String, String)> {
        let mut checksums = Vec::new();
        for href in hrefs.iter().filter(|href| is_checksum_sidecar(href)) {
            let Ok(url) = self.scoped_url(base, href) else {
                continue;
            };
            let Ok(response) = self.client.get(url).send().await else {
                continue;
            };
            let Ok(text) = response.text().await else {
                continue;
            };
            let mut entries = parse_checksums(&text);
            if entries.is_empty() {
                let hash = text.trim();
                if valid_sha256(hash) {
                    entries.push((sidecar_artifact_name(href), hash.to_ascii_lowercase()));
                }
            }
            checksums.extend(entries);
        }
        checksums
    }

    async fn file_sha256(&self, url: &Url) -> Option<String> {
        let response = self.client.get(url.clone()).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let mut hash = Sha256::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            hash.update(chunk.ok()?);
        }
        Some(format!("{:x}", hash.finalize()))
    }

    fn relative_path(&self, url: &Url) -> String {
        url.path()
            .strip_prefix(self.root.path())
            .map(|path| format!("/{path}"))
            .unwrap_or_else(|| url.path().to_owned())
    }
}

async fn persist_results(
    state: &AppState,
    scanner: &Scanner,
    results: Vec<ScanResult>,
) -> Result<(), String> {
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "database is unavailable".to_owned())?;
    let mut seen = HashSet::new();
    for result in results {
        seen.insert((result.folder_name.clone(), result.version.clone()));
        let folder = sqlx::query_as::<_, (String, String)>(
            r#"SELECT "deviceType", "deviceFamily" FROM device_type_folders
               WHERE "folderName" = $1"#,
        )
        .bind(&result.folder_name)
        .fetch_optional(database.pool())
        .await
        .map_err(|error| error.to_string())?;
        let Some((device_type, device_family)) = folder else {
            tracing::warn!(folder = %result.folder_name, version = %result.version, "skipping artifact folder without a device mapping");
            continue;
        };
        let artifacts =
            serde_json::to_value(&result.artifacts).map_err(|error| error.to_string())?;
        let existing = sqlx::query_as::<_, (Uuid, String, String, serde_json::Value)>(
            r#"SELECT id, "uploadedBy", "buildType"::text, artifacts
               FROM releases WHERE "folderName" = $1 AND version = $2"#,
        )
        .bind(&result.folder_name)
        .bind(&result.version)
        .fetch_optional(database.pool())
        .await
        .map_err(|error| error.to_string())?;
        if let Some((release_id, uploaded_by, build_type, existing_artifacts)) = existing {
            sqlx::query(
                r#"UPDATE releases SET "lastScannedAt" = now(), "updatedAt" = now()
                   WHERE id = $1"#,
            )
            .bind(release_id)
            .execute(database.pool())
            .await
            .map_err(|error| error.to_string())?;
            if uploaded_by != "system"
                || (!artifact_metadata_changed(&existing_artifacts, &result.artifacts)
                    && required_local_artifacts_exist(&existing_artifacts))
            {
                continue;
            }
            match prepare_scanned_build(
                state,
                scanner,
                &result,
                release_id,
                &device_type,
                &device_family,
                build_type == "official",
            )
            .await
            {
                Ok(local_artifacts) => {
                    sqlx::query(
                        r#"UPDATE releases SET artifacts = $2, "updatedAt" = now()
                           WHERE id = $1"#,
                    )
                    .bind(release_id)
                    .bind(local_artifacts)
                    .execute(database.pool())
                    .await
                    .map_err(|error| error.to_string())?;
                }
                Err(error) => {
                    tracing::error!(%error, folder = %result.folder_name, version = %result.version, "failed to refresh scanned build");
                }
            }
            continue;
        }
        let release_id = Uuid::new_v4();
        let build_type = if is_custom_version(&result.version) {
            "custom"
        } else {
            "official"
        };
        sqlx::query(
            r#"INSERT INTO releases
                  (id, "folderName", version, artifacts, "lastScannedAt", tag,
                   "uploadedBy", "buildType", "deviceFamily", "deviceType",
                   "createdAt", "updatedAt")
               VALUES ($1, $2, $3, $4, now(), $5, 'system',
                       $6::"enum_releases_buildType", $7, $8, now(), now())
               ON CONFLICT ("folderName", version) DO NOTHING"#,
        )
        .bind(release_id)
        .bind(&result.folder_name)
        .bind(&result.version)
        .bind(&artifacts)
        .bind(format!(
            "system-{}",
            &Uuid::new_v4().simple().to_string()[..8]
        ))
        .bind(build_type)
        .bind(&device_family)
        .bind(&device_type)
        .execute(database.pool())
        .await
        .map_err(|error| error.to_string())?;
        match prepare_scanned_build(
            state,
            scanner,
            &result,
            release_id,
            &device_type,
            &device_family,
            build_type == "official",
        )
        .await
        {
            Ok(local_artifacts) => {
                sqlx::query(
                    r#"UPDATE releases SET artifacts = $2, "updatedAt" = now()
                       WHERE id = $1"#,
                )
                .bind(release_id)
                .bind(local_artifacts)
                .execute(database.pool())
                .await
                .map_err(|error| error.to_string())?;
            }
            Err(error) => {
                tracing::error!(%error, folder = %result.folder_name, version = %result.version, "failed to prepare scanned build");
            }
        }
        let _ = state.event_publisher.publish(ServerEvent {
            event: "build_uploaded".to_owned(),
            payload: serde_json::json!({
                "releaseId": release_id,
                "version": result.version,
                "deviceType": device_type,
                "deviceFamily": device_family,
                "buildType": build_type,
            }),
            room: None,
        });
    }
    cleanup_removed(state, &seen).await
}

async fn prepare_scanned_build(
    state: &AppState,
    scanner: &Scanner,
    result: &ScanResult,
    release_id: Uuid,
    device_type: &str,
    device_family: &str,
    official: bool,
) -> Result<serde_json::Value, String> {
    let required = required_artifacts(&result.artifacts)?;
    let folder_name = safe_segment(&result.folder_name)?.to_owned();
    let version = safe_segment(&result.version)?.to_owned();
    let destination = PathBuf::from(&state.config.device.build_artifacts_dir)
        .join(&folder_name)
        .join(&version);
    let parent = destination
        .parent()
        .ok_or_else(|| "artifact destination has no parent".to_owned())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(".scan-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&temporary)
        .await
        .map_err(|error| error.to_string())?;
    let outcome = async {
        let _downloaded = [
            download(
                &scanner.client,
                &required[0].source_url,
                temporary.join("Image"),
            )
            .await?,
            download(
                &scanner.client,
                &required[1].source_url,
                temporary.join("board.dtb"),
            )
            .await?,
            download(
                &scanner.client,
                &required[2].source_url,
                temporary.join("root.tar.bz2"),
            )
            .await?,
        ];
        for artifact in result
            .artifacts
            .iter()
            .filter(|artifact| artifact.name.starts_with("ipl/"))
        {
            let relative = safe_relative_artifact_path(&artifact.name)?;
            let destination = temporary.join(relative);
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            download(&scanner.client, &artifact.source_url, destination).await?;
        }
        let backup = parent.join(format!(".previous-{}", Uuid::new_v4()));
        let had_destination = tokio::fs::try_exists(&destination)
            .await
            .map_err(|error| error.to_string())?;
        if had_destination {
            tokio::fs::rename(&destination, &backup)
                .await
                .map_err(|error| error.to_string())?;
        }
        if let Err(error) = tokio::fs::rename(&temporary, &destination).await {
            if had_destination {
                let _ = tokio::fs::rename(&backup, &destination).await;
            }
            return Err(error.to_string());
        }
        if had_destination {
            let _ = tokio::fs::remove_dir_all(backup).await;
        }
        let paths = [
            destination.join("Image"),
            destination.join("board.dtb"),
            destination.join("root.tar.bz2"),
        ];
        let device_type = safe_segment(device_type)?.to_owned();
        let nfs = PathBuf::from(&state.config.device.nfs_host_path)
            .join(&device_type)
            .join(&version);
        let tftp = PathBuf::from(&state.config.device.tftp_output_dir)
            .join(&device_type)
            .join(&version);
        let preparation_paths = paths.clone();
        tokio::task::spawn_blocking(move || prepare_artifacts(&preparation_paths, &nfs, &tftp))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        if official {
            enqueue_official_build(state, release_id, &device_type, &version, "system").await?;
        }
        if let Err(error) = super::ipl_distribution::prepare_and_distribute(
            state,
            &paths,
            device_family,
            &version,
        )
        .await
        {
            tracing::error!(%error, device_family, version, "failed to prepare scanned IPL payload");
        }
        let mut local_artifacts = result.artifacts.clone();
        for artifact in &mut local_artifacts {
            if let Some(path) = local_artifact_path(&destination, &artifact.name) {
                artifact.path = path.to_string_lossy().into_owned();
            }
        }
        serde_json::to_value(local_artifacts).map_err(|error| error.to_string())
    }
    .await;
    let _ = tokio::fs::remove_dir_all(temporary).await;
    outcome
}

fn local_artifact_path(destination: &std::path::Path, name: &str) -> Option<PathBuf> {
    if name.eq_ignore_ascii_case("Image") {
        Some(destination.join("Image"))
    } else if name.to_ascii_lowercase().ends_with(".dtb") {
        Some(destination.join("board.dtb"))
    } else if name.to_ascii_lowercase().ends_with(".tar.bz2") {
        Some(destination.join("root.tar.bz2"))
    } else if name.starts_with("ipl/") {
        safe_relative_artifact_path(name)
            .ok()
            .map(|path| destination.join(path))
    } else {
        None
    }
}

fn safe_relative_artifact_path(name: &str) -> Result<PathBuf, String> {
    let path = std::path::Path::new(name);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(format!("unsafe scanned artifact path: {name}"));
    }
    Ok(path.to_path_buf())
}

fn required_local_artifacts_exist(artifacts: &serde_json::Value) -> bool {
    let Some(artifacts) = artifacts.as_array() else {
        return false;
    };
    let path_for = |predicate: &dyn Fn(&str) -> bool| {
        artifacts.iter().find_map(|artifact| {
            let name = artifact.get("name")?.as_str()?;
            predicate(name).then(|| artifact.get("path")?.as_str())?
        })
    };
    let required_boot_artifacts_exist = [
        path_for(&|name| name.eq_ignore_ascii_case("Image")),
        path_for(&|name| name.to_ascii_lowercase().ends_with(".dtb")),
        path_for(&|name| name.to_ascii_lowercase().ends_with(".tar.bz2")),
    ]
    .into_iter()
    .all(|path| path.is_some_and(|path| std::path::Path::new(path).is_file()));
    required_boot_artifacts_exist
        && artifacts
            .iter()
            .filter(|artifact| {
                artifact
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|name| name.starts_with("ipl/"))
            })
            .all(|artifact| {
                artifact
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|path| std::path::Path::new(path).is_file())
            })
}

fn artifact_metadata_changed(existing: &serde_json::Value, scanned: &[ArtifactInfo]) -> bool {
    let Some(existing) = existing.as_array() else {
        return true;
    };
    if existing.len() != scanned.len() {
        return true;
    }
    scanned.iter().any(|artifact| {
        existing
            .iter()
            .find(|current| {
                current.get("name").and_then(serde_json::Value::as_str) == Some(&artifact.name)
            })
            .is_none_or(|current| {
                current.get("size").and_then(serde_json::Value::as_u64) != Some(artifact.size)
                    || current.get("sha256").and_then(serde_json::Value::as_str)
                        != artifact.sha256.as_deref()
            })
    })
}

async fn download(client: &Client, url: &Url, destination: PathBuf) -> Result<PathBuf, String> {
    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let mut file = tokio::fs::File::create(&destination)
        .await
        .map_err(|error| error.to_string())?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk.map_err(|error| error.to_string())?)
            .await
            .map_err(|error| error.to_string())?;
    }
    file.flush().await.map_err(|error| error.to_string())?;
    Ok(destination)
}

async fn cleanup_removed(state: &AppState, seen: &HashSet<(String, String)>) -> Result<(), String> {
    let cleanup = state.config.workers.artifact_scan_cleanup_removed;
    if cleanup == ArtifactCleanup::Off {
        return Ok(());
    }
    let database = state.database.as_ref().expect("worker requires database");
    let rows = sqlx::query_as::<_, (Uuid, String, String)>(
        r#"SELECT id, "folderName", version FROM releases
           WHERE "uploadedBy" = 'system' AND "lastScannedAt" IS NOT NULL"#,
    )
    .fetch_all(database.pool())
    .await
    .map_err(|error| error.to_string())?;
    for (id, folder, version) in rows {
        if seen.contains(&(folder, version)) {
            continue;
        }
        match cleanup {
            ArtifactCleanup::Soft => {
                sqlx::query(r#"UPDATE releases SET "lastScannedAt" = NULL, "updatedAt" = now() WHERE id = $1"#)
                    .bind(id)
                    .execute(database.pool())
                    .await
                    .map_err(|error| error.to_string())?;
            }
            ArtifactCleanup::Delete => {
                sqlx::query("DELETE FROM releases WHERE id = $1")
                    .bind(id)
                    .execute(database.pool())
                    .await
                    .map_err(|error| error.to_string())?;
            }
            ArtifactCleanup::Off => {}
        }
    }
    Ok(())
}

fn required_artifacts(artifacts: &[ArtifactInfo]) -> Result<[&ArtifactInfo; 3], String> {
    let image = artifacts
        .iter()
        .find(|artifact| artifact.name.eq_ignore_ascii_case("Image"))
        .ok_or_else(|| "release Image artifact is missing".to_owned())?;
    let dtb = artifacts
        .iter()
        .find(|artifact| artifact.name.to_ascii_lowercase().ends_with(".dtb"))
        .ok_or_else(|| "release DTB artifact is missing".to_owned())?;
    let rootfs = artifacts
        .iter()
        .find(|artifact| artifact.name.to_ascii_lowercase().ends_with(".tar.bz2"))
        .ok_or_else(|| "release rootfs artifact is missing".to_owned())?;
    Ok([image, dtb, rootfs])
}

fn final_path_segment(url: &Url) -> Result<String, String> {
    let segment = url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .ok_or_else(|| format!("URL has no path segment: {url}"))?;
    percent_encoding::percent_decode_str(segment)
        .decode_utf8()
        .map(|value| value.into_owned())
        .map_err(|error| error.to_string())
}

fn is_custom_version(version: &str) -> bool {
    version.split_once('-').is_some_and(|(_, suffix)| {
        suffix
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
    })
}

fn is_checksum_sidecar(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".sha256")
        || lower.ends_with(".sha256.txt")
        || lower.ends_with(".sha256sum")
        || lower.ends_with(".sha256sum.txt")
}

fn sidecar_artifact_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for suffix in [".sha256sum.txt", ".sha256.txt", ".sha256sum", ".sha256"] {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].to_owned();
        }
    }
    name.to_owned()
}

fn parse_checksums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            (valid_sha256(hash) && !name.is_empty()).then(|| {
                let name = name.rsplit('/').next().unwrap_or(name).to_owned();
                (name, hash.to_ascii_lowercase())
            })
        })
        .collect()
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;

    #[test]
    fn parses_checksum_formats_and_build_types() {
        let hash = "A".repeat(64);
        assert_eq!(
            parse_checksums(&format!("{hash} *nested/Image\n")),
            vec![("Image".to_owned(), hash.to_ascii_lowercase())]
        );
        assert!(is_custom_version("v1.2.3-nightly"));
        assert!(!is_custom_version("v1.2.3"));
        assert_eq!(sidecar_artifact_name("Image.sha256.txt"), "Image");
    }

    #[tokio::test]
    async fn scans_structured_directory_indexes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/artifacts/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("<a href=\"Gen5_x5h/\">x5h</a>"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/artifacts/Gen5_x5h/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<a href=\"v1.2.3/\">v1</a>"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/artifacts/Gen5_x5h/v1.2.3/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<a href=\"Image\">Image</a><a href=\"board.dtb\">dtb</a><a href=\"root.tar.bz2\">root</a><a href=\"ipl/\">ipl</a>",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/artifacts/Gen5_x5h/v1.2.3/ipl/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<a href=\"boot.tar.gz\">boot</a><a href=\"hil/\">hil</a>"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/artifacts/Gen5_x5h/v1.2.3/ipl/hil/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("<a href=\"config.json\">config</a>"),
            )
            .mount(&server)
            .await;
        for name in ["Image", "board.dtb", "root.tar.bz2"] {
            Mock::given(method("HEAD"))
                .and(path(format!("/artifacts/Gen5_x5h/v1.2.3/{name}")))
                .respond_with(ResponseTemplate::new(200).insert_header("content-length", "4"))
                .mount(&server)
                .await;
        }
        for path_value in [
            "/artifacts/Gen5_x5h/v1.2.3/ipl/boot.tar.gz",
            "/artifacts/Gen5_x5h/v1.2.3/ipl/hil/config.json",
        ] {
            Mock::given(method("HEAD"))
                .and(path(path_value))
                .respond_with(ResponseTemplate::new(200).insert_header("content-length", "4"))
                .mount(&server)
                .await;
        }
        let scanner = Scanner::new(&format!("{}/artifacts", server.uri()), 2, false).unwrap();
        let results = scanner.scan_once().await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].folder_name, "Gen5_x5h");
        assert_eq!(results[0].version, "v1.2.3");
        assert_eq!(results[0].artifacts.len(), 5);
        assert_eq!(results[0].artifacts[0].path, "/Gen5_x5h/v1.2.3/Image");
        assert!(
            results[0]
                .artifacts
                .iter()
                .any(|artifact| artifact.name == "ipl/hil/config.json")
        );
    }

    #[test]
    fn scanner_refresh_requires_current_local_artifacts() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["Image", "board.dtb", "root.tar.bz2"] {
            std::fs::write(directory.path().join(name), name).unwrap();
        }
        std::fs::create_dir(directory.path().join("ipl")).unwrap();
        std::fs::write(directory.path().join("ipl/boot.tar.gz"), b"ipl").unwrap();
        let existing = serde_json::json!([
            {"name": "Image", "size": 4, "path": directory.path().join("Image")},
            {"name": "board.dtb", "size": 4, "path": directory.path().join("board.dtb")},
            {"name": "root.tar.bz2", "size": 4, "path": directory.path().join("root.tar.bz2")},
            {"name": "ipl/boot.tar.gz", "size": 3, "path": directory.path().join("ipl/boot.tar.gz")}
        ]);
        assert!(required_local_artifacts_exist(&existing));
        std::fs::remove_file(directory.path().join("ipl/boot.tar.gz")).unwrap();
        assert!(!required_local_artifacts_exist(&existing));
        std::fs::write(directory.path().join("ipl/boot.tar.gz"), b"ipl").unwrap();
        std::fs::remove_file(directory.path().join("Image")).unwrap();
        assert!(!required_local_artifacts_exist(&existing));
    }
}
