use super::backend::{ChunkWriteResult, StorageBackend, StorageError, VerifyResult};
use bytes::Bytes;
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;

#[derive(Clone)]
pub struct LocalStorageBackend {
    root: PathBuf,
}

impl LocalStorageBackend {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn object_path(&self, object_key: &str) -> Result<PathBuf, StorageError> {
        let parts: Vec<&str> = object_key.split('/').collect();
        match parts.as_slice() {
            ["files", id, "chunks", chunk_index] | ["uploads", id, "chunks", chunk_index] => {
                if !is_safe_id(id) || !is_safe_chunk_index(chunk_index) {
                    return Err(StorageError::InvalidObjectKey);
                }
                Ok(self
                    .root
                    .join(parts[0])
                    .join(id)
                    .join("chunks")
                    .join(chunk_index))
            }
            _ => Err(StorageError::InvalidObjectKey),
        }
    }

    pub fn path_for_object(&self, object_key: &str) -> Result<PathBuf, StorageError> {
        self.object_path(object_key)
    }
}

impl StorageBackend for LocalStorageBackend {
    fn put_chunk<'a>(
        &'a self,
        object_key: &'a str,
        bytes: Bytes,
        expected_sha256: &'a str,
    ) -> BoxFuture<'a, Result<ChunkWriteResult, StorageError>> {
        Box::pin(async move {
            validate_sha256(expected_sha256)?;
            let path = self.object_path(object_key)?;
            let actual_sha256 = sha256_hex(&bytes);
            if actual_sha256 != expected_sha256 {
                return Err(StorageError::ChecksumMismatch {
                    expected: expected_sha256.to_string(),
                    actual: actual_sha256,
                });
            }

            if let Ok(existing) = fs::read(&path).await {
                let existing_sha256 = sha256_hex(&existing);
                if existing_sha256 == expected_sha256 {
                    return Ok(ChunkWriteResult {
                        size_bytes: existing.len() as u64,
                        sha256: existing_sha256,
                    });
                }
                return Err(StorageError::Conflict(format!(
                    "object key {object_key} already exists with different checksum"
                )));
            }

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await?;
            }

            let tmp_path = temp_path(&path);
            let mut file = File::create(&tmp_path).await?;
            file.write_all(&bytes).await?;
            file.flush().await?;
            drop(file);
            fs::rename(&tmp_path, &path).await?;

            Ok(ChunkWriteResult {
                size_bytes: bytes.len() as u64,
                sha256: expected_sha256.to_string(),
            })
        })
    }

    fn open_chunk<'a>(&'a self, object_key: &'a str) -> BoxFuture<'a, Result<File, StorageError>> {
        Box::pin(async move {
            let path = self.object_path(object_key)?;
            match File::open(path).await {
                Ok(file) => Ok(file),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    Err(StorageError::NotFound)
                }
                Err(err) => Err(StorageError::Io(err)),
            }
        })
    }

    fn delete_chunk<'a>(&'a self, object_key: &'a str) -> BoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            let path = self.object_path(object_key)?;
            match fs::remove_file(path).await {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(StorageError::Io(err)),
            }
        })
    }

    fn verify_chunk<'a>(
        &'a self,
        object_key: &'a str,
        expected_sha256: &'a str,
    ) -> BoxFuture<'a, Result<VerifyResult, StorageError>> {
        Box::pin(async move {
            validate_sha256(expected_sha256)?;
            let path = self.object_path(object_key)?;
            let bytes = match fs::read(path).await {
                Ok(bytes) => bytes,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(VerifyResult {
                        exists: false,
                        valid: false,
                        size_bytes: 0,
                        sha256: None,
                    });
                }
                Err(err) => return Err(StorageError::Io(err)),
            };
            let actual_sha256 = sha256_hex(&bytes);
            Ok(VerifyResult {
                exists: true,
                valid: actual_sha256 == expected_sha256,
                size_bytes: bytes.len() as u64,
                sha256: Some(actual_sha256),
            })
        })
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("chunk");
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.with_file_name(format!(".{name}.tmp-{unique}"))
}

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn is_safe_chunk_index(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn validate_sha256(value: &str) -> Result<(), StorageError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(StorageError::InvalidObjectKey)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}
