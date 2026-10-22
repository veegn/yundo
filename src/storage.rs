//! Staged writes keep quota reservations until publication or completed cleanup.
use crate::{errors::AppError, state::CacheUsageTracker};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
    sync::OwnedRwLockReadGuard,
};

pub struct PendingFile {
    file: Option<File>,
    path: PathBuf,
    tracker: Arc<CacheUsageTracker>,
    limit: u64,
    reserved: u64,
    operation: Option<Arc<OwnedRwLockReadGuard<()>>>,
    committed: bool,
}

impl PendingFile {
    pub async fn create(
        path: PathBuf,
        tracker: Arc<CacheUsageTracker>,
        limit: u64,
        operation: Arc<OwnedRwLockReadGuard<()>>,
    ) -> Result<Self, AppError> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await?;
        Ok(Self {
            file: Some(file),
            path,
            tracker,
            limit,
            reserved: 0,
            operation: Some(operation),
            committed: false,
        })
    }

    pub fn size(&self) -> u64 {
        self.reserved
    }

    pub async fn write_all(&mut self, bytes: &[u8]) -> Result<(), AppError> {
        self.tracker
            .try_reserve(bytes.len() as u64, self.limit)
            .map_err(|_| AppError::StorageFull)?;
        self.reserved += bytes.len() as u64;
        self.file
            .as_mut()
            .expect("pending file closed")
            .write_all(bytes)
            .await?;
        Ok(())
    }

    pub async fn rename(&mut self, destination: &Path) -> Result<(), AppError> {
        if let Some(mut file) = self.file.take() {
            file.flush().await?;
            drop(file);
        }
        // Keep the rename and ownership update in one poll: cancellation between
        // them must not release quota for a file that already moved elsewhere.
        std::fs::rename(&self.path, destination)?;
        self.path = destination.to_path_buf();
        Ok(())
    }

    pub fn commit(mut self) {
        self.committed = true;
    }

    pub async fn discard(mut self) {
        self.file.take();
        self.cleanup().await;
        self.committed = true;
    }

    async fn cleanup(&self) {
        match fs::remove_file(&self.path).await {
            Ok(()) => self.tracker.sub(self.reserved),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.tracker.sub(self.reserved)
            }
            Err(error) => {
                tracing::warn!(path = %self.path.display(), %error, "staged file cleanup failed; retaining quota")
            }
        }
    }
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let file = self.file.take();
        let path = self.path.clone();
        let tracker = self.tracker.clone();
        let reserved = self.reserved;
        let operation = self.operation.take();
        // Also covers client disconnects and cancelled request futures. Keep the
        // operation guard until disk cleanup finishes so calibration cannot race.
        tokio::spawn(async move {
            drop(file);
            match fs::remove_file(&path).await {
                Ok(()) => tracker.sub(reserved),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => tracker.sub(reserved),
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "cancelled upload cleanup failed; retaining quota")
                }
            }
            drop(operation);
        });
    }
}

/// Only release bytes that have actually left the disk.
pub async fn remove_accounted_file(
    path: &Path,
    tracker: &CacheUsageTracker,
) -> std::io::Result<()> {
    let size = match fs::metadata(path).await {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    fs::remove_file(path).await?;
    tracker.sub(size);
    Ok(())
}

pub async fn remove_accounted_directory(
    path: &Path,
    tracker: &CacheUsageTracker,
) -> std::io::Result<()> {
    let mut entries = match fs::read_dir(path).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    while let Some(entry) = entries.next_entry().await? {
        // Upload directories contain only numbered chunks, never nested directories.
        if entry.file_type().await?.is_file() {
            remove_accounted_file(&entry.path(), tracker).await?;
        }
    }
    fs::remove_dir(path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_staged_write_removes_payload_before_releasing_guard() {
        let directory = tempfile::TempDir::new().unwrap();
        let tracker = Arc::new(CacheUsageTracker::new());
        let operation = Arc::new(tracker.operations.clone().read_owned().await);
        let path = directory.path().join("upload.tmp");
        let mut file = PendingFile::create(path.clone(), tracker.clone(), 8, operation.clone())
            .await
            .unwrap();
        file.write_all(b"abcdef").await.unwrap();
        assert_eq!(tracker.get(), 6);
        drop(operation);
        drop(file);
        let _idle = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tracker.operations.write(),
        )
        .await
        .unwrap();
        assert!(!path.exists());
        assert_eq!(tracker.get(), 0);
    }
}
