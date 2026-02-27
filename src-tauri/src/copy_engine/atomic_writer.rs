//! Atomic Writer — Ensures files are either fully written or not present at all.
//!
//! Flow:
//! 1. Write to a temporary file (.tmp suffix)
//! 2. Flush and sync to disk
//! 3. Atomic rename to final path
//!
//! If interrupted at any point, only the .tmp file remains, which is
//! automatically cleaned up on recovery.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// An atomic file writer that writes to .tmp then renames on completion.
/// Implements Drop to automatically clean up temp files on cancellation.
pub struct AtomicWriter {
    temp_path: PathBuf,
    final_path: PathBuf,
    file: Option<tokio::fs::File>,
    bytes_written: u64,
    /// Set to true after finalize() succeeds, so Drop doesn't delete the renamed file
    finalized: bool,
}

impl AtomicWriter {
    /// Create a new atomic writer. The file will be written to `path.tmp`
    /// and renamed to `path` on successful finalization.
    pub async fn new(final_path: &Path) -> Result<Self> {
        let temp_path = Self::temp_path_for(final_path);

        // Ensure parent directory exists
        if let Some(parent) = temp_path.parent() {
            fs::create_dir_all(parent).await.with_context(|| {
                format!("Failed to create parent directory: {:?}", parent)
            })?;
        }

        let file = fs::File::create(&temp_path).await.with_context(|| {
            format!("Failed to create temp file: {:?}", temp_path)
        })?;

        Ok(Self {
            temp_path,
            final_path: final_path.to_path_buf(),
            file: Some(file),
            bytes_written: 0,
            finalized: false,
        })
    }

    /// Write a chunk of data to the temporary file
    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        let file = self.file.as_mut().context("AtomicWriter already consumed")?;
        file.write_all(data).await.with_context(|| {
            format!("Failed to write to temp file: {:?}", self.temp_path)
        })?;
        self.bytes_written += data.len() as u64;
        Ok(())
    }

    /// Flush, sync to disk, and atomically rename to the final path.
    /// This is the point of no return — after this call succeeds,
    /// the file is guaranteed to be complete and correctly named.
    pub async fn finalize(mut self) -> Result<()> {
        if let Some(mut file) = self.file.take() {
            // Flush internal buffers
            file.flush().await?;
            // Sync to disk (ensures data is physically written)
            file.sync_all().await?;
            // Drop the file handle before rename
            drop(file);
        }

        // Atomic rename
        fs::rename(&self.temp_path, &self.final_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to rename {:?} -> {:?}",
                    self.temp_path, self.final_path
                )
            })?;

        self.finalized = true;
        Ok(())
    }

    /// Abort the write and clean up the temporary file
    pub async fn abort(mut self) -> Result<()> {
        // Drop file handle first
        self.file.take();
        if self.temp_path.exists() {
            fs::remove_file(&self.temp_path).await.ok();
        }
        self.finalized = true; // Prevent Drop from trying cleanup again
        Ok(())
    }

    /// Get the temporary file path for a given final path
    pub fn temp_path_for(final_path: &Path) -> PathBuf {
        let mut temp = final_path.as_os_str().to_owned();
        temp.push(".tmp");
        PathBuf::from(temp)
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    pub fn temp_path(&self) -> &Path {
        &self.temp_path
    }
}

/// Drop implementation to clean up temp files when AtomicWriter is dropped
/// without calling finalize() or abort() (e.g., on cancellation/panic).
impl Drop for AtomicWriter {
    fn drop(&mut self) {
        if !self.finalized {
            // Use synchronous fs to clean up — Drop cannot be async.
            // Best-effort: ignore errors during cleanup.
            let _ = std::fs::remove_file(&self.temp_path);
        }
    }
}

/// Clean up any orphaned .tmp files in a directory (used during recovery)
pub async fn cleanup_tmp_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut cleaned = Vec::new();
    let mut entries = fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "tmp" {
                    fs::remove_file(&path).await.with_context(|| {
                        format!("Failed to clean up tmp file: {:?}", path)
                    })?;
                    cleaned.push(path);
                }
            }
        }
    }

    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn test_atomic_write_success() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("test_file.mov");

        // Write atomically
        let mut writer = AtomicWriter::new(&final_path).await.unwrap();
        writer.write(b"hello ").await.unwrap();
        writer.write(b"world").await.unwrap();
        assert_eq!(writer.bytes_written(), 11);

        // Before finalize: .tmp exists, final doesn't
        assert!(writer.temp_path().exists());
        assert!(!final_path.exists());

        writer.finalize().await.unwrap();

        // After finalize: final exists, .tmp doesn't
        assert!(final_path.exists());
        assert!(!AtomicWriter::temp_path_for(&final_path).exists());

        // Verify content
        let mut f = tokio::fs::File::open(&final_path).await.unwrap();
        let mut content = String::new();
        f.read_to_string(&mut content).await.unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_atomic_write_abort() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("aborted_file.mov");

        let mut writer = AtomicWriter::new(&final_path).await.unwrap();
        writer.write(b"partial data").await.unwrap();
        let temp_path = writer.temp_path().to_path_buf();

        writer.abort().await.unwrap();

        // Neither file should exist
        assert!(!final_path.exists());
        assert!(!temp_path.exists());
    }

    #[tokio::test]
    async fn test_cleanup_tmp_files() {
        let dir = tempfile::tempdir().unwrap();

        // Create some .tmp files and a normal file
        tokio::fs::write(dir.path().join("file1.mov.tmp"), b"orphan1").await.unwrap();
        tokio::fs::write(dir.path().join("file2.r3d.tmp"), b"orphan2").await.unwrap();
        tokio::fs::write(dir.path().join("real_file.mov"), b"keep me").await.unwrap();

        let cleaned = cleanup_tmp_files(dir.path()).await.unwrap();
        assert_eq!(cleaned.len(), 2);

        // .tmp files removed, real file kept
        assert!(!dir.path().join("file1.mov.tmp").exists());
        assert!(!dir.path().join("file2.r3d.tmp").exists());
        assert!(dir.path().join("real_file.mov").exists());
    }
}
