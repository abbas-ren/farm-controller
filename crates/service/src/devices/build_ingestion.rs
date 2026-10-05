use std::{
    fs::File,
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

pub(crate) fn extract_zip(
    archive_path: &Path,
    target: &Path,
    expected_root: &str,
    device_family: Option<&str>,
    max_expanded_bytes: u64,
) -> io::Result<Vec<serde_json::Value>> {
    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(io::Error::other)?;
    let family = device_family
        .unwrap_or_default()
        .replace(' ', "")
        .to_lowercase();
    let requires_ipl = matches!(family.as_str(), "gen4" | "gen5");
    let mut image_count = 0;
    let mut dtb_count = 0;
    let mut tar_count = 0;
    let mut gen4_ipl_count = 0;
    let mut gen5_ipl_count = 0;
    let mut expanded = 0_u64;
    let mut artifacts = Vec::new();
    std::fs::create_dir_all(target)?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(io::Error::other)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(invalid("Symbolic links are not allowed in build ZIPs"));
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| invalid("Expanded ZIP size overflow"))?;
        if expanded > max_expanded_bytes {
            return Err(invalid("Expanded ZIP exceeds configured maximum size"));
        }
        let normalized = normalize_entry(entry.name())?;
        let Some(relative) = strip_root(&normalized, expected_root) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        let is_directory = entry.is_dir();
        if !is_directory {
            validate_layout(
                &relative_text,
                &family,
                requires_ipl,
                &mut image_count,
                &mut dtb_count,
                &mut tar_count,
                &mut gen4_ipl_count,
                &mut gen5_ipl_count,
            )?;
        }
        let output = target.join(&relative);
        if is_directory {
            std::fs::create_dir_all(&output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output_file = File::create(&output)?;
        io::copy(&mut entry, &mut output_file)?;
        output_file.flush()?;
        if !relative_text.contains('/') {
            artifacts.push(serde_json::json!({
                "name": relative.file_name().and_then(|name| name.to_str()).unwrap_or_default(),
                "size": entry.size(),
                "path": output.to_string_lossy(),
                "mtime": chrono::Utc::now(),
            }));
        }
    }

    if image_count != 1 || dtb_count != 1 || tar_count != 1 {
        return Err(invalid(
            "Zip must contain root Image, board.dtb, and exactly one root .tar.bz2",
        ));
    }
    if family == "gen5" && gen5_ipl_count != 1 {
        return Err(invalid(
            "Zip must contain exactly one direct .tar.gz file under ipl/",
        ));
    }
    if family == "gen4" && gen4_ipl_count < 1 {
        return Err(invalid(
            "Gen4 zip must contain at least one file directly under ipl/",
        ));
    }
    Ok(artifacts)
}

fn normalize_entry(name: &str) -> io::Result<PathBuf> {
    let replaced = name.replace('\\', "/");
    if replaced.starts_with('/') {
        return Err(invalid("Invalid path inside zip"));
    }
    let path = Path::new(&replaced);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(invalid("Invalid path inside zip"));
    }
    Ok(path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect())
}

fn strip_root(path: &Path, expected_root: &str) -> Option<PathBuf> {
    let mut components = path.components();
    let first = components.next();
    if first
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .is_some_and(|value| value.eq_ignore_ascii_case(expected_root))
    {
        Some(components.collect())
    } else {
        Some(path.to_path_buf())
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_layout(
    relative: &str,
    family: &str,
    requires_ipl: bool,
    image_count: &mut usize,
    dtb_count: &mut usize,
    tar_count: &mut usize,
    gen4_ipl_count: &mut usize,
    gen5_ipl_count: &mut usize,
) -> io::Result<()> {
    let lower = relative.to_lowercase();
    let base = Path::new(relative)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !relative.contains('/') {
        if base.eq_ignore_ascii_case("image") {
            *image_count += 1;
        } else if lower.ends_with(".dtb") {
            *dtb_count += 1;
        } else if lower.ends_with(".tar.bz2") {
            *tar_count += 1;
        } else {
            return Err(invalid(&format!("Unexpected file: {relative}")));
        }
        return Ok(());
    }
    if !lower.starts_with("ipl/") || !requires_ipl {
        return Err(invalid(&format!("Unexpected file: {relative}")));
    }
    let nested = relative[4..].contains('/');
    if family == "gen4" {
        if nested {
            return Err(invalid(
                "Gen4 zip must contain only files directly under ipl/ (no subfolders)",
            ));
        }
        *gen4_ipl_count += 1;
    } else if !nested && lower.ends_with(".tar.gz") {
        *gen5_ipl_count += 1;
    } else if !lower.starts_with("ipl/hil/") {
        return Err(invalid(&format!("Unexpected file: {relative}")));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;

    fn archive(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = ZipWriter::new(file);
        for (name, bytes) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn extracts_legacy_root_layout() {
        let directory = tempfile::tempdir().unwrap();
        let zip = directory.path().join("x5h__1.2.3.zip");
        archive(
            &zip,
            &[
                ("x5h__1.2.3/Image", b"image"),
                ("x5h__1.2.3/board.dtb", b"dtb"),
                ("x5h__1.2.3/root.tar.bz2", b"root"),
                ("x5h__1.2.3/ipl/boot.tar.gz", b"ipl"),
            ],
        );
        let output = directory.path().join("output");
        let artifacts = extract_zip(&zip, &output, "x5h__1.2.3", Some("Gen5"), 1024).unwrap();
        assert_eq!(artifacts.len(), 3);
        assert!(output.join("Image").is_file());
    }

    #[test]
    fn rejects_parent_paths_and_expansion_limit() {
        let directory = tempfile::tempdir().unwrap();
        let zip = directory.path().join("unsafe.zip");
        archive(&zip, &[("../Image", b"image")]);
        assert!(extract_zip(&zip, &directory.path().join("out"), "unsafe", None, 1024).is_err());

        let zip = directory.path().join("large.zip");
        archive(&zip, &[("Image", b"too large")]);
        assert!(extract_zip(&zip, &directory.path().join("out2"), "large", None, 2).is_err());
    }
}
