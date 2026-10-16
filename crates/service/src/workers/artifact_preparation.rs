use std::{
    error::Error,
    fs::{self, File, Permissions},
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use bzip2::read::MultiBzDecoder;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub(crate) fn artifact_paths(
    value: &serde_json::Value,
    rootfs_archive_extension: &str,
) -> Result<[PathBuf; 3], String> {
    let artifacts = value
        .as_array()
        .ok_or_else(|| "release artifacts are not an array".to_owned())?;

    let find = |predicate: &dyn Fn(&str) -> bool| {
        artifacts.iter().find_map(|artifact| {
            let name = artifact.get("name")?.as_str()?;
            predicate(name).then(|| artifact.get("path")?.as_str().map(PathBuf::from))?
        })
    };

    let image = find(&|name| name.eq_ignore_ascii_case("Image"))
        .ok_or_else(|| "release Image artifact is missing".to_owned())?;
    let dtb = find(&|name| name.to_ascii_lowercase().ends_with(".dtb"))
        .ok_or_else(|| "release DTB artifact is missing".to_owned())?;
    let extension = rootfs_archive_extension.to_ascii_lowercase();
    let rootfs = find(&|name| name.to_ascii_lowercase().ends_with(&extension))
        .ok_or_else(|| "release rootfs artifact is missing".to_owned())?;

    Ok([image, dtb, rootfs])
}

pub(crate) fn prepare_artifacts(
    paths: &[PathBuf; 3],
    nfs_path: &Path,
    tftp_path: &Path,
) -> io::Result<()> {
    if !nfs_path.join(".prep-done").is_file() {
        let parent = nfs_path
            .parent()
            .ok_or_else(|| io::Error::other("NFS target has no parent"))?;

        fs::create_dir_all(parent)?;

        // Build in the same parent filesystem so the final rename is atomic.
        let staging = parent.join(format!(".prep-{}", Uuid::new_v4()));
        fs::create_dir(&staging)?;

        let result = (|| {
            let archive_file = File::open(&paths[2]).map_err(|error| {
                with_context(
                    error,
                    format!("opening rootfs archive {}", paths[2].display()),
                )
            })?;
            let decoder = MultiBzDecoder::new(archive_file);
            unpack_rootfs(tar::Archive::new(decoder), &staging)?;

            fs::write(staging.join(".prep-done"), chrono::Utc::now().to_rfc3339())?;

            // Do not delete the previous target until a complete staging tree
            // has been prepared. Rename is atomic when source and target share
            // a filesystem; replacing a non-empty directory is not portable.
            if nfs_path.exists() {
                fs::remove_dir_all(nfs_path)?;
            }
            fs::rename(&staging, nfs_path)
        })();

        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result?;
    }

    fs::create_dir_all(tftp_path)?;
    fs::copy(&paths[0], tftp_path.join("Image")).map_err(|error| {
        with_context(
            error,
            format!("copying kernel image {}", paths[0].display()),
        )
    })?;
    fs::copy(&paths[1], tftp_path.join("board.dtb")).map_err(|error| {
        with_context(error, format!("copying device tree {}", paths[1].display()))
    })?;

    Ok(())
}

#[derive(Debug)]
struct DirectoryMetadata {
    relative_path: PathBuf,
    mode: u32,
    mtime: u64,
}

#[derive(Debug)]
struct PendingHardLink {
    relative_path: PathBuf,
    relative_target: PathBuf,
}

fn unpack_rootfs<R: io::Read>(mut archive: tar::Archive<R>, destination: &Path) -> io::Result<()> {
    let destination = destination.canonicalize()?;
    let mut directories = Vec::<DirectoryMetadata>::new();
    let mut hard_links = Vec::<PendingHardLink>::new();

    // Important: never retain tar::Entry values for later. Entries share the
    // archive's streaming reader; advancing the iterator and then unpacking a
    // retained entry can leave that reader at the wrong position.
    for entry_result in archive.entries()? {
        let mut entry = entry_result
            .map_err(|error| with_context(error, "reading next rootfs archive entry".to_owned()))?;

        let entry_type = entry.header().entry_type();

        if entry_type.is_dir() {
            let relative_path = normalize_archive_path(entry.path()?.as_ref(), true)?;
            let mode = entry.header().mode()?;
            let mtime = entry.header().mtime()?;

            // Extract the directory now so ownership and other metadata handled
            // by the tar crate are retained. Make it temporarily traversable;
            // restore its archived mode and timestamp after all children exist.
            entry.unpack_in(&destination).map_err(|error| {
                with_context(
                    error,
                    format!("unpacking rootfs directory {}", relative_path.display()),
                )
            })?;

            let directory_path = if relative_path.as_os_str().is_empty() {
                destination.clone()
            } else {
                destination.join(&relative_path)
            };
            fs::set_permissions(&directory_path, temporary_directory_permissions())?;

            directories.push(DirectoryMetadata {
                relative_path,
                mode,
                mtime,
            });
        } else if entry_type.is_hard_link() {
            let relative_path = normalize_archive_path(entry.path()?.as_ref(), false)?;
            let link_name = entry
                .link_name()?
                .ok_or_else(|| io::Error::other("rootfs hard link has no target"))?;
            let relative_target = normalize_archive_path(link_name.as_ref(), false)?;

            // The link entry has no file payload. Save only owned path values;
            // create the link once its target has been extracted.
            hard_links.push(PendingHardLink {
                relative_path,
                relative_target,
            });
        } else {
            let path = entry.path()?.into_owned();
            let size = entry.header().size()?;

            entry.unpack_in(&destination).map_err(|error| {
                with_context(
                    error,
                    format!(
                        "unpacking rootfs entry {} (declared size {size} bytes)",
                        path.display()
                    ),
                )
            })?;
        }
    }

    create_hard_links(&destination, hard_links)?;
    restore_directory_metadata(&destination, directories)?;

    Ok(())
}

fn create_hard_links(destination: &Path, mut pending: Vec<PendingHardLink>) -> io::Result<()> {
    while !pending.is_empty() {
        let previous_count = pending.len();
        let mut next_round = Vec::new();
        let mut first_missing = None;

        for link in pending {
            match create_one_hard_link(destination, &link) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    first_missing.get_or_insert(error);
                    next_round.push(link);
                }
                Err(error) => return Err(error),
            }
        }

        if next_round.len() == previous_count {
            let unresolved = next_round
                .first()
                .map(|link| {
                    format!(
                        "unresolved rootfs hard link {} -> {}",
                        link.relative_path.display(),
                        link.relative_target.display()
                    )
                })
                .unwrap_or_else(|| "rootfs contains an unresolved hard link".to_owned());

            return Err(first_missing
                .map(|error| with_context(error, unresolved.clone()))
                .unwrap_or_else(|| io::Error::other(unresolved)));
        }

        pending = next_round;
    }

    Ok(())
}

fn create_one_hard_link(destination: &Path, link: &PendingHardLink) -> io::Result<()> {
    let source = destination
        .join(&link.relative_target)
        .canonicalize()
        .map_err(|error| {
            with_context(
                error,
                format!(
                    "resolving hard-link target {}",
                    link.relative_target.display()
                ),
            )
        })?;

    if !source.starts_with(destination) {
        return Err(io::Error::other(
            "rootfs hard-link target escapes extraction root",
        ));
    }

    let output = destination.join(&link.relative_path);
    let parent = output
        .parent()
        .ok_or_else(|| io::Error::other("rootfs hard link has no parent"))?;

    fs::create_dir_all(parent)?;
    let canonical_parent = parent.canonicalize()?;
    if !canonical_parent.starts_with(destination) {
        return Err(io::Error::other(
            "rootfs hard-link path escapes extraction root",
        ));
    }

    // A duplicate archive path should replace the prior extracted entry.
    // Remove only after validating the destination parent.
    match fs::symlink_metadata(&output) {
        Ok(_) => {
            if fs::symlink_metadata(&output)?.file_type().is_dir() {
                fs::remove_dir_all(&output)?;
            } else {
                fs::remove_file(&output)?;
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    fs::hard_link(source, output).map_err(|error| {
        with_context(
            error,
            format!(
                "creating rootfs hard link {} -> {}",
                link.relative_path.display(),
                link.relative_target.display()
            ),
        )
    })
}

fn restore_directory_metadata(
    destination: &Path,
    mut directories: Vec<DirectoryMetadata>,
) -> io::Result<()> {
    // Restore deepest directories first. Parent timestamps are restored last,
    // after all child creation and metadata updates are complete.
    directories
        .sort_by_key(|directory| std::cmp::Reverse(directory.relative_path.components().count()));

    for directory in directories {
        let path = if directory.relative_path.as_os_str().is_empty() {
            destination.to_path_buf()
        } else {
            destination.join(&directory.relative_path)
        };

        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_dir() {
            return Err(io::Error::other(format!(
                "expected rootfs directory at {}",
                path.display()
            )));
        }

        set_modified_time(&path, directory.mtime)?;
        fs::set_permissions(&path, permissions_from_mode(directory.mode))?;
    }

    Ok(())
}

fn set_modified_time(path: &Path, seconds: u64) -> io::Result<()> {
    let modified = UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| io::Error::other("rootfs directory timestamp is out of range"))?;
    let file = File::open(path)?;
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
}

#[cfg(unix)]
fn temporary_directory_permissions() -> Permissions {
    Permissions::from_mode(0o755)
}

#[cfg(unix)]
fn permissions_from_mode(mode: u32) -> Permissions {
    Permissions::from_mode(mode)
}

fn normalize_archive_path(path: &Path, allow_empty: bool) -> io::Result<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            _ => {
                return Err(io::Error::other(format!(
                    "rootfs archive contains unsafe path: {}",
                    path.display()
                )));
            }
        }
    }

    if normalized.as_os_str().is_empty() && !allow_empty {
        return Err(io::Error::other(
            "rootfs archive link contains an empty path",
        ));
    }

    Ok(normalized)
}

fn with_context(error: io::Error, context: String) -> io::Error {
    let kind = error.kind();
    let mut message = context;
    message.push_str(": ");
    message.push_str(&error.to_string());

    let mut source = error.source();
    while let Some(cause) = source {
        let cause_message = cause.to_string();
        if !message.contains(&cause_message) {
            message.push_str(": ");
            message.push_str(&cause_message);
        }
        source = cause.source();
    }

    io::Error::new(kind, message)
}

pub(crate) fn safe_segment(value: &str) -> Result<&str, String> {
    let mut components = Path::new(value).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(value),
        _ => Err(format!("unsafe path segment: {value}")),
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Write, os::unix::fs::MetadataExt};

    use bzip2::{Compression, write::BzEncoder};
    use tar::EntryType;

    use super::*;

    fn create_archive(path: &Path, entries: impl FnOnce(&mut tar::Builder<BzEncoder<File>>)) {
        let encoder = BzEncoder::new(File::create(path).unwrap(), Compression::default());
        let mut archive = tar::Builder::new(encoder);
        entries(&mut archive);
        archive
            .into_inner()
            .unwrap()
            .finish()
            .unwrap()
            .flush()
            .unwrap();
    }

    #[test]
    fn stages_release_for_nfs_and_tftp() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("source-image");
        let dtb = directory.path().join("source.dtb");
        let rootfs = directory.path().join("root.tar.bz2");
        fs::write(&image, b"image").unwrap();
        fs::write(&dtb, b"dtb").unwrap();

        create_archive(&rootfs, |archive| {
            let mut header = tar::Header::new_gnu();
            header.set_size(4);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, "etc/release", &b"root"[..])
                .unwrap();
        });

        let nfs = directory.path().join("nfs/x5h/v1.2.3");
        let tftp = directory.path().join("tftp/x5h/v1.2.3");
        prepare_artifacts(&[image, dtb, rootfs], &nfs, &tftp).unwrap();

        assert_eq!(fs::read(nfs.join("etc/release")).unwrap(), b"root");
        assert!(nfs.join(".prep-done").is_file());
        assert_eq!(fs::read(tftp.join("Image")).unwrap(), b"image");
        assert_eq!(fs::read(tftp.join("board.dtb")).unwrap(), b"dtb");
    }

    #[test]
    fn stages_rootfs_with_hard_link_before_target() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("source-image");
        let dtb = directory.path().join("source.dtb");
        let rootfs = directory.path().join("root.tar.bz2");
        fs::write(&image, b"image").unwrap();
        fs::write(&dtb, b"dtb").unwrap();

        create_archive(&rootfs, |archive| {
            let mut link = tar::Header::new_gnu();
            link.set_entry_type(EntryType::Link);
            link.set_size(0);
            link.set_mode(0o755);
            archive
                .append_link(&mut link, "bin/bash.bash", "bin/bash")
                .unwrap();

            let mut target = tar::Header::new_gnu();
            target.set_size(4);
            target.set_mode(0o755);
            target.set_cksum();
            archive
                .append_data(&mut target, "bin/bash", &b"bash"[..])
                .unwrap();
        });

        let nfs = directory.path().join("nfs/v4h/v1.2.3");
        let tftp = directory.path().join("tftp/v4h/v1.2.3");
        prepare_artifacts(&[image, dtb, rootfs], &nfs, &tftp).unwrap();

        assert_eq!(fs::read(nfs.join("bin/bash.bash")).unwrap(), b"bash");
        assert_eq!(
            fs::metadata(nfs.join("bin/bash.bash")).unwrap().ino(),
            fs::metadata(nfs.join("bin/bash")).unwrap().ino()
        );
    }

    #[test]
    fn rejects_unsafe_archive_paths() {
        assert!(normalize_archive_path(Path::new("../escape"), false).is_err());
        assert!(normalize_archive_path(Path::new("/absolute"), false).is_err());
        assert_eq!(
            normalize_archive_path(Path::new("./etc/release"), false).unwrap(),
            PathBuf::from("etc/release")
        );
    }

    #[test]
    fn rejects_unsafe_path_segments() {
        assert_eq!(safe_segment("x5h").unwrap(), "x5h");
        assert!(safe_segment("../x5h").is_err());
        assert!(safe_segment("x5h/version").is_err());
    }
}
