use bytes::Bytes;
use futures_util::future::BoxFuture;
use tokio::fs::File;

#[derive(Debug, Clone)]
pub struct ChunkWriteResult {
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub exists: bool,
    pub valid: bool,
    pub size_bytes: u64,
    pub sha256: Option<String>,
}

#[derive(Debug)]
pub enum StorageError {
    InvalidObjectKey,
    ChecksumMismatch { expected: String, actual: String },
    Conflict(String),
    NotFound,
    Io(std::io::Error),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::InvalidObjectKey => write!(f, "invalid object key"),
            StorageError::ChecksumMismatch { expected, actual } => {
                write!(f, "checksum mismatch: expected {expected}, actual {actual}")
            }
            StorageError::Conflict(message) => write!(f, "storage conflict: {message}"),
            StorageError::NotFound => write!(f, "object not found"),
            StorageError::Io(err) => write!(f, "storage io error: {err}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(value: std::io::Error) -> Self {
        StorageError::Io(value)
    }
}

pub trait StorageBackend: Send + Sync {
    fn put_chunk<'a>(
        &'a self,
        object_key: &'a str,
        bytes: Bytes,
        expected_sha256: &'a str,
    ) -> BoxFuture<'a, Result<ChunkWriteResult, StorageError>>;

    fn open_chunk<'a>(&'a self, object_key: &'a str) -> BoxFuture<'a, Result<File, StorageError>>;

    fn delete_chunk<'a>(&'a self, object_key: &'a str) -> BoxFuture<'a, Result<(), StorageError>>;

    fn verify_chunk<'a>(
        &'a self,
        object_key: &'a str,
        expected_sha256: &'a str,
    ) -> BoxFuture<'a, Result<VerifyResult, StorageError>>;
}
