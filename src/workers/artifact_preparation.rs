use std::{
    fs::File,
    io,
    path::{Component, Path, PathBuf},
};

use bzip2::read::BzDecoder;
use uuid::Uuid;

pub(crate) fn artifact_paths(value: &serde_json::Value) -> Result<[PathBuf; 3], String> {
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
    let rootfs = find(&|name| name.to_ascii_lowercase().ends_with(".tar.bz2"))
        .ok_or_else(|| "release rootfs artifact is missing".to_owned())?;
    Ok([image, dtb, rootfs])
}

pub(crate) fn prepare_artifacts(
    paths: &[PathBuf; 3],
    nfs_path: &Path,
    tftp_path: &Path,
) -> io::Result<()> {
    if !nfs_path.join(".prep-done").is_file() {
        if nfs_path.exists() {
            std::fs::remove_dir_all(nfs_path)?;
        }
        let parent = nfs_path
            .parent()
            .ok_or_else(|| io::Error::other("NFS target has no parent"))?;
        std::fs::create_dir_all(parent)?;
        let staging = parent.join(format!(".prep-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&staging)?;
        let result = (|| {
            let decoder = BzDecoder::new(File::open(&paths[2])?);
            tar::Archive::new(decoder).unpack(&staging)?;
            std::fs::write(staging.join(".prep-done"), chrono::Utc::now().to_rfc3339())?;
            std::fs::rename(&staging, nfs_path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        result?;
    }
    std::fs::create_dir_all(tftp_path)?;
    std::fs::copy(&paths[0], tftp_path.join("Image"))?;
    std::fs::copy(&paths[1], tftp_path.join("board.dtb"))?;
    Ok(())
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
    use std::io::Write;

    use bzip2::{Compression, write::BzEncoder};

    use super::*;

    #[test]
    fn stages_release_for_nfs_and_tftp() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("source-image");
        let dtb = directory.path().join("source.dtb");
        let rootfs = directory.path().join("root.tar.bz2");
        std::fs::write(&image, b"image").unwrap();
        std::fs::write(&dtb, b"dtb").unwrap();
        let encoder = BzEncoder::new(File::create(&rootfs).unwrap(), Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "etc/release", &b"root"[..])
            .unwrap();
        archive
            .into_inner()
            .unwrap()
            .finish()
            .unwrap()
            .flush()
            .unwrap();

        let nfs = directory.path().join("nfs/x5h/v1.2.3");
        let tftp = directory.path().join("tftp/x5h/v1.2.3");
        prepare_artifacts(&[image, dtb, rootfs], &nfs, &tftp).unwrap();

        assert_eq!(std::fs::read(nfs.join("etc/release")).unwrap(), b"root");
        assert!(nfs.join(".prep-done").is_file());
        assert_eq!(std::fs::read(tftp.join("Image")).unwrap(), b"image");
        assert_eq!(std::fs::read(tftp.join("board.dtb")).unwrap(), b"dtb");
    }

    #[test]
    fn rejects_unsafe_path_segments() {
        assert_eq!(safe_segment("x5h").unwrap(), "x5h");
        assert!(safe_segment("../x5h").is_err());
        assert!(safe_segment("x5h/version").is_err());
    }
}
