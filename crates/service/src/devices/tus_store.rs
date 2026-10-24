//! On-disk TUS resumable-upload storage (metadata JSON + binary data file).
//!
//! Used by `tus_handlers` for create / HEAD / PATCH of build uploads before
//! finalization hands off to `build_store`. Replaces the Node tus-node-server
//! disk store used by `BuildService` upload endpoints.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// In-progress TUS upload metadata persisted beside the binary blob.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TusUpload {
    /// Stable upload id (also the data filename).
    pub id: Uuid,
    /// Declared total byte length from `Upload-Length`.
    pub length: u64,
    /// Bytes successfully written so far (TUS `Upload-Offset`).
    pub offset: u64,
    /// Client metadata (filename, filetype, …) from `Upload-Metadata`.
    pub metadata: BTreeMap<String, String>,
}

/// PURPOSE: Create an empty data file and metadata record for a new upload.
/// USE: TUS POST create handler after validating length / metadata headers.
pub(crate) async fn create(
    root: &Path,
    length: u64,
    metadata: BTreeMap<String, String>,
) -> std::io::Result<TusUpload> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_store", "create");
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

/// PURPOSE: Load upload metadata from `{id}.json`.
/// USE: HEAD / PATCH handlers to report or verify current offset.
pub(crate) async fn load(root: &Path, id: Uuid) -> std::io::Result<TusUpload> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_store", "load");
    let bytes = tokio::fs::read(metadata_path(root, id)).await?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

/// PURPOSE: Append a chunk at the expected offset under an exclusive lock file.
/// USE: TUS PATCH; rejects offset mismatches and oversize chunks.
pub(crate) async fn append(
    root: &Path,
    id: Uuid,
    expected_offset: u64,
    bytes: &[u8],
) -> std::io::Result<TusUpload> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_store", "append");
    // Exclusive create-new lock prevents concurrent PATCH races on the same upload.
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

/// PURPOSE: Truncate the data file and metadata offset after a failed PATCH.
/// USE: TUS error recovery when a partial write must be discarded.
pub(crate) async fn rollback(root: &Path, id: Uuid, offset: u64) -> std::io::Result<()> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_store", "rollback");
    let mut upload = load(root, id).await?;
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(data_path(root, id))
        .await?;
    file.set_len(offset).await?;
    upload.offset = offset;
    save(root, &upload).await
}

/// PURPOSE: Atomically write metadata via temp file + rename.
/// USE: After create or successful append.
async fn save(root: &Path, upload: &TusUpload) -> std::io::Result<()> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_store", "save");
    let path = metadata_path(root, upload.id);
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(upload).map_err(std::io::Error::other)?;
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::rename(temporary, path).await
}

/// PURPOSE: Path to the binary blob for an upload id.
/// USE: Finalization reads this file; PATCH appends to it.
pub(crate) fn data_path(root: &Path, id: Uuid) -> PathBuf {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_store", "data_path");
    root.join(id.to_string())
}

/// PURPOSE: Path to the JSON metadata sidecar.
/// USE: Internal load/save helpers.
fn metadata_path(root: &Path, id: Uuid) -> PathBuf {
    let _api_trace = crate::observability::ApiTrace::enter("devices::tus_store", "metadata_path");
    root.join(format!("{id}.json"))
}

#[cfg(test)]
mod tests {
    //! Round-trip coverage for create / append / reload offset.

    use super::*;

    /// PURPOSE: Ensure create/append offsets persist across metadata reload.
    /// USE: Unit test for TUS store durability of `Upload-Offset`.
    #[tokio::test]
    async fn upload_offset_survives_storage_reload() {
        let _api_trace = crate::observability::ApiTrace::enter("devices::tus_store", "upload_offset_survives_storage_reload");
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
