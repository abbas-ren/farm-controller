//! Derive eagent boot paths from a device's registered NFS root and sync test scripts.
//!
//! Mode A (reuse current root): keep the DUT on its registered NFS/TFTP prefixes and only
//! ensure `usr/src/` test scripts exist there. Mode B (full image): use a fresh
//! `{deviceType}/{testId}` tree prepared from the full release ZIP.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

/// Absolute NFS root plus absolute-style TFTP `image` / `dtb` paths for eagent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BootPaths {
    pub nfs: String,
    pub image: String,
    pub dtb: String,
}

/// PURPOSE: Map a registered NFS root under `nfs_host_path` to TFTP Image/DTB paths.
/// USE: Mode A test-execution dispatch when reusing the DUT's current boot prefixes.
///
/// Expects `{nfs_host_path}/{deviceType}/{segment}` → `/{deviceType}/{segment}/Image|board.dtb`.
pub(crate) fn derive_boot_paths(nfs_host_path: &str, nfs_path: &str) -> Result<BootPaths, String> {
    let host = normalize_abs(nfs_host_path).ok_or_else(|| {
        "device.nfs_host_path must be an absolute path without '..'".to_owned()
    })?;
    let nfs = normalize_abs(nfs_path)
        .ok_or_else(|| "device nfsPath must be an absolute path without '..'".to_owned())?;
    if nfs == host {
        return Err("device nfsPath must be a build directory under nfs_host_path".to_owned());
    }
    let relative = nfs
        .strip_prefix(&host)
        .map(|value| value.trim_start_matches('/'))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!("device nfsPath '{nfs}' is not under nfs_host_path '{host}'")
        })?;
    if relative.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(format!("device nfsPath relative segment is unsafe: {relative}"));
    }
    // Convention: exactly deviceType/segment (matches farm TFTP layout).
    let mut parts = relative.split('/');
    let device_type = parts.next().filter(|part| !part.is_empty());
    let segment = parts.next().filter(|part| !part.is_empty());
    let trailing = parts.next();
    if device_type.is_none() || segment.is_none() || trailing.is_some() {
        return Err(format!(
            "device nfsPath must look like {{nfs_host}}/{{deviceType}}/{{segment}}, got '{nfs}'"
        ));
    }
    Ok(BootPaths {
        nfs: nfs.clone(),
        image: format!("/{relative}/Image"),
        dtb: format!("/{relative}/board.dtb"),
    })
}

/// PURPOSE: Copy prepared `usr/src` (scripts + markers) onto the DUT's current NFS root.
/// USE: Mode A dispatch after test_preparation staged scripts under `{type}/{testId}`.
pub(crate) fn sync_usr_src_tree(staging_root: &Path, current_nfs_root: &Path) -> io::Result<()> {
    let source = staging_root.join("usr").join("src");
    if !source.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "staged test scripts are missing at {}",
                source.display()
            ),
        ));
    }
    let destination = current_nfs_root.join("usr").join("src");
    copy_dir_recursive(&source, &destination)
}

/// PURPOSE: Build the per-test staging NFS directory used by test_preparation.
/// USE: Mode A sync source and Mode B currentTest.txt location.
pub(crate) fn staging_nfs_root(
    nfs_host_path: &str,
    device_type: &str,
    test_id: &str,
) -> PathBuf {
    PathBuf::from(nfs_host_path.trim_end_matches('/'))
        .join(device_type)
        .join(test_id)
}

fn normalize_abs(path: &str) -> Option<String> {
    let trimmed = path.trim().trim_end_matches('/');
    if !trimmed.starts_with('/') || trimmed.contains('\0') {
        return None;
    }
    let mut parts = Vec::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => {
                let text = part.to_str()?;
                if text == "." || text == ".." {
                    return None;
                }
                parts.push(text);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    Some(format!("/{}", parts.join("/")))
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if file_type.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_tftp_paths_from_registered_nfs() {
        let paths = derive_boot_paths("/nfs_share", "/nfs_share/x5h/v1.2.3").unwrap();
        assert_eq!(
            paths,
            BootPaths {
                nfs: "/nfs_share/x5h/v1.2.3".to_owned(),
                image: "/x5h/v1.2.3/Image".to_owned(),
                dtb: "/x5h/v1.2.3/board.dtb".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_nfs_outside_host_or_wrong_depth() {
        assert!(derive_boot_paths("/nfs_share", "/other/x5h/v1").is_err());
        assert!(derive_boot_paths("/nfs_share", "/nfs_share/x5h").is_err());
        assert!(derive_boot_paths("/nfs_share", "/nfs_share/x5h/v1/extra").is_err());
        assert!(derive_boot_paths("/nfs_share", "../nfs_share/x5h/v1").is_err());
    }

    #[test]
    fn syncs_usr_src_onto_current_root() {
        let directory = tempfile::tempdir().unwrap();
        let staging = directory.path().join("staging");
        let current = directory.path().join("current");
        let script_dir = staging.join("usr/src/tests/test-1");
        fs::create_dir_all(&script_dir).unwrap();
        fs::write(script_dir.join("10.sh"), b"echo ok").unwrap();
        fs::write(staging.join("usr/src/currentTest.txt"), b"test-1").unwrap();

        sync_usr_src_tree(&staging, &current).unwrap();

        assert_eq!(
            fs::read(current.join("usr/src/tests/test-1/10.sh")).unwrap(),
            b"echo ok"
        );
        assert_eq!(
            fs::read(current.join("usr/src/currentTest.txt")).unwrap(),
            b"test-1"
        );
    }
}
