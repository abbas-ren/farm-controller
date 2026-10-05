use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TusUpload {
    pub id: Uuid,
    pub length: u64,
    pub offset: u64,
    pub metadata: BTreeMap<String, String>,
}

pub(crate) async fn create(
    root: &Path,
    length: u64,
    metadata: BTreeMap<String, String>,
) -> std::io::Result<TusUpload> {
    tokio::fs::create_dir_all(root).await?;
    let upload = TusUpload {
        id: Uuid::new_v4(),
        length,
        offset: 0,
        metadata,
    };
    tokio::fs::File::create(data_path(root, upload.id)).await?;
    save(root, &upload).await?;
    Ok(upload)
}

pub(crate) async fn load(root: &Path, id: Uuid) -> std::io::Result<TusUpload> {
    let bytes = tokio::fs::read(metadata_path(root, id)).await?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

pub(crate) async fn append(
    root: &Path,
    id: Uuid,
    expected_offset: u64,
    bytes: &[u8],
) -> std::io::Result<TusUpload> {
    let lock_path = root.join(format!("{id}.lock"));
    let _lock = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .await?;
    let result = async {
        let mut upload = load(root, id).await?;
        if upload.offset != expected_offset {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "upload offset mismatch",
            ));
        }
        if upload.offset.saturating_add(bytes.len() as u64) > upload.length {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "chunk exceeds upload length",
            ));
        }
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(data_path(root, upload.id))
            .await?;
        file.write_all(bytes).await?;
        file.flush().await?;
        upload.offset += bytes.len() as u64;
        save(root, &upload).await?;
        Ok(upload)
    }
    .await;
    let _ = tokio::fs::remove_file(lock_path).await;
    result
}

pub(crate) async fn rollback(root: &Path, id: Uuid, offset: u64) -> std::io::Result<()> {
    let mut upload = load(root, id).await?;
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(data_path(root, id))
        .await?;
    file.set_len(offset).await?;
    upload.offset = offset;
    save(root, &upload).await
}

async fn save(root: &Path, upload: &TusUpload) -> std::io::Result<()> {
    let path = metadata_path(root, upload.id);
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(upload).map_err(std::io::Error::other)?;
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::rename(temporary, path).await
}

pub(crate) fn data_path(root: &Path, id: Uuid) -> PathBuf {
    root.join(id.to_string())
}

fn metadata_path(root: &Path, id: Uuid) -> PathBuf {
    root.join(format!("{id}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upload_offset_survives_storage_reload() {
        let directory = tempfile::tempdir().unwrap();
        let upload = create(
            directory.path(),
            5,
            BTreeMap::from([("filename".to_owned(), "build.bin".to_owned())]),
        )
        .await
        .unwrap();
        let upload = append(directory.path(), upload.id, 0, b"abc")
            .await
            .unwrap();
        assert_eq!(upload.offset, 3);
        assert_eq!(load(directory.path(), upload.id).await.unwrap().offset, 3);
        assert_eq!(
            tokio::fs::read(data_path(directory.path(), upload.id))
                .await
                .unwrap(),
            b"abc"
        );
    }
}
