use std::{
    io,
    path::{Component, Path, PathBuf},
};

use futures_util::TryStreamExt;
use tokio::io::AsyncWriteExt;
use tokio_util::io::StreamReader;

use crate::config::DeviceConfig;

use super::{DeviceTypeFolder, constants::DEVICE_CALLBACK_TIMEOUT};

pub(crate) async fn discover_versions(base_url: &str, folder_keyword: &str) -> Vec<String> {
    let Ok(base_url) = reqwest::Url::parse(base_url) else {
        return Vec::new();
    };
    let Ok(client) = crate::external_http::client(DEVICE_CALLBACK_TIMEOUT) else {
        return Vec::new();
    };
    let Ok(root_response) = client.get(base_url.clone()).send().await else {
        return Vec::new();
    };
    let Ok(root_html) = root_response.text().await else {
        return Vec::new();
    };
    let Some(folder) = html_directories(&root_html).into_iter().find(|folder| {
        folder
            .to_lowercase()
            .contains(&folder_keyword.to_lowercase())
    }) else {
        return Vec::new();
    };
    let Ok(folder_url) = base_url.join(&format!("{folder}/")) else {
        return Vec::new();
    };
    let Ok(folder_response) = client.get(folder_url).send().await else {
        return Vec::new();
    };
    folder_response
        .text()
        .await
        .map_or_else(|_| Vec::new(), |html| html_directories(&html))
}

fn html_directories(html: &str) -> Vec<String> {
    let link = regex::Regex::new(r#"(?i)href\s*=\s*["']([^"']+/)["']"#)
        .expect("static directory link regex is valid");
    link.captures_iter(html)
        .filter_map(|capture| {
            capture
                .get(1)
                .map(|value| value.as_str().trim_end_matches('/'))
        })
        .filter(|directory| *directory != ".." && !directory.contains('/'))
        .map(str::to_owned)
        .collect()
}

pub(crate) async fn copy_defaults(
    config: &DeviceConfig,
    entries: &[DeviceTypeFolder],
    force: bool,
) -> Result<(), String> {
    let base_url = reqwest::Url::parse(&config.artifacts_base_url)
        .map_err(|error| format!("invalid artifacts base URL: {error}"))?;
    let client = crate::external_http::client(std::time::Duration::from_secs(
        config.request_timeout_seconds,
    ))
    .map_err(|error| error.to_string())?;
    let root_html = get_text(&client, base_url.clone()).await?;
    let folders = html_directories(&root_html);
    for entry in entries {
        validate_segment(&entry.device_type)?;
        validate_segment(&entry.folder_name)?;
        validate_segment(&entry.default_version)?;
        if !folders.contains(&entry.folder_name) {
            tracing::warn!(folder = %entry.folder_name, device_type = %entry.device_type, "default artifact folder is unavailable");
            continue;
        }
        let version_url = base_url
            .join(&format!("{}/{}/", entry.folder_name, entry.default_version))
            .map_err(|error| format!("invalid artifact version URL: {error}"))?;
        let files = html_files(&get_text(&client, version_url.clone()).await?);
        let image = files.iter().find(|file| file.as_str() == "Image");
        let dtb = files.iter().find(|file| file.ends_with(".dtb"));
        let archive = files.iter().find(|file| file.ends_with(".tar.bz2"));
        let (Some(image), Some(dtb), Some(archive)) = (image, dtb, archive) else {
            tracing::warn!(folder = %entry.folder_name, version = %entry.default_version, "default artifact set is incomplete");
            continue;
        };
        let staging = Path::new(&config.artifacts_temp_dir).join(staging_name());
        tokio::fs::create_dir_all(&staging)
            .await
            .map_err(io_error)?;
        let install = install_entry(
            &client,
            &version_url,
            &staging,
            entry,
            image,
            dtb,
            archive,
            config,
            force,
        )
        .await;
        if let Err(error) = tokio::fs::remove_dir_all(&staging).await {
            tracing::warn!(%error, path = %staging.display(), "failed to clean artifact staging directory");
        }
        install?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn install_entry(
    client: &reqwest::Client,
    version_url: &reqwest::Url,
    staging: &Path,
    entry: &DeviceTypeFolder,
    image: &str,
    dtb: &str,
    archive: &str,
    config: &DeviceConfig,
    force: bool,
) -> Result<(), String> {
    for file in [image, dtb, archive] {
        validate_segment(file)?;
        download(
            client,
            version_url.join(file).map_err(|error| error.to_string())?,
            &staging.join(file),
        )
        .await?;
    }
    let nfs = Path::new(&config.nfs_host_path)
        .join(&entry.device_type)
        .join(&entry.default_version);
    let tftp = Path::new(&config.tftp_output_dir)
        .join(&entry.device_type)
        .join(&entry.default_version);
    tokio::fs::create_dir_all(&nfs).await.map_err(io_error)?;
    tokio::fs::create_dir_all(&tftp).await.map_err(io_error)?;
    if force || directory_is_empty(&nfs).await? {
        if force {
            replace_directory(&nfs).await?;
        }
        extract_archive(staging.join(archive), nfs.clone()).await?;
    }
    copy_if_needed(&staging.join(image), &tftp.join("Image"), force).await?;
    copy_if_needed(&staging.join(dtb), &tftp.join("board.dtb"), force).await
}

async fn get_text(client: &reqwest::Client, url: reqwest::Url) -> Result<String, String> {
    client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())
}

async fn download(client: &reqwest::Client, url: reqwest::Url, path: &Path) -> Result<(), String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let mut reader = StreamReader::new(response.bytes_stream().map_err(io::Error::other));
    let mut file = tokio::fs::File::create(path).await.map_err(io_error)?;
    tokio::io::copy(&mut reader, &mut file)
        .await
        .map_err(io_error)?;
    file.flush().await.map_err(io_error)
}

async fn extract_archive(archive: PathBuf, destination: PathBuf) -> Result<(), String> {
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let file = std::fs::File::open(archive).map_err(io_error)?;
        let mut archive = tar::Archive::new(bzip2::read::BzDecoder::new(file));
        for entry in archive.entries().map_err(io_error)? {
            let mut entry = entry.map_err(io_error)?;
            let path = entry.path().map_err(io_error)?;
            if path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            }) {
                return Err("artifact archive contains an unsafe path".to_owned());
            }
            entry.unpack_in(&destination).map_err(io_error)?;
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

async fn directory_is_empty(path: &Path) -> Result<bool, String> {
    Ok(tokio::fs::read_dir(path)
        .await
        .map_err(io_error)?
        .next_entry()
        .await
        .map_err(io_error)?
        .is_none())
}

async fn replace_directory(path: &Path) -> Result<(), String> {
    tokio::fs::remove_dir_all(path).await.map_err(io_error)?;
    tokio::fs::create_dir_all(path).await.map_err(io_error)
}

async fn copy_if_needed(source: &Path, destination: &Path, force: bool) -> Result<(), String> {
    if force || !tokio::fs::try_exists(destination).await.map_err(io_error)? {
        tokio::fs::copy(source, destination)
            .await
            .map_err(io_error)?;
    }
    Ok(())
}

fn html_files(html: &str) -> Vec<String> {
    let link = regex::Regex::new(r#"(?i)href\s*=\s*["']([^"'/]+)["']"#)
        .expect("static file link regex is valid");
    link.captures_iter(html)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str()))
        .filter(|file| *file != ".." && !file.contains(['/', '\\']))
        .map(str::to_owned)
        .collect()
}

fn validate_segment(segment: &str) -> Result<(), String> {
    let path = Path::new(segment);
    if segment.is_empty()
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(format!("unsafe artifact path segment: {segment}"));
    }
    Ok(())
}

fn io_error(error: io::Error) -> String {
    error.to_string()
}

fn staging_name() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::{html_directories, html_files, validate_segment};

    #[test]
    fn directory_listing_ignores_parent_and_files() {
        let html = r#"<a href="../">parent</a><a href="v1/">v1</a><a href="Image">file</a>"#;
        assert_eq!(html_directories(html), vec!["v1"]);
    }

    #[test]
    fn file_listing_keeps_only_single_path_segments() {
        let html = r#"<a href="../">parent</a><a href="Image">image</a><a href="board.dtb">dtb</a><a href="nested/file">bad</a>"#;
        assert_eq!(html_files(html), vec!["Image", "board.dtb"]);
    }

    #[test]
    fn artifact_path_segments_reject_traversal() {
        assert!(validate_segment("device-v1").is_ok());
        for segment in ["", "..", "../escape", "/absolute", "nested/path"] {
            assert!(validate_segment(segment).is_err(), "segment: {segment}");
        }
    }
}
