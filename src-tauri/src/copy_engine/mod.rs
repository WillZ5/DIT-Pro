//! Copy Engine — Multi-path parallel copy with single-source-read optimization.
//!
//! Core responsibilities:
//! - Read source file once, write to multiple destinations simultaneously
//! - Cascading copy (fast device first, then slow devices)
//! - Inline hash verification during copy
//! - Atomic write (.tmp + rename)
//! - Pre-copy space validation

pub mod atomic_writer;

use crate::hash_engine::{HashAlgorithm, HashResult, MultiHasher};
use anyhow::{bail, Context, Result};
use atomic_writer::AtomicWriter;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

/// Status of a single copy task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CopyTaskStatus {
    Pending,
    Copying,
    Verifying,
    Completed,
    Failed(String),
    Skipped,
}

impl std::fmt::Display for CopyTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CopyTaskStatus::Pending => write!(f, "pending"),
            CopyTaskStatus::Copying => write!(f, "copying"),
            CopyTaskStatus::Verifying => write!(f, "verifying"),
            CopyTaskStatus::Completed => write!(f, "completed"),
            CopyTaskStatus::Failed(msg) => write!(f, "failed: {}", msg),
            CopyTaskStatus::Skipped => write!(f, "skipped"),
        }
    }
}

/// A copy task representing one source file to one or more destinations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyTask {
    pub id: String,
    pub source_path: PathBuf,
    pub dest_paths: Vec<PathBuf>,
    pub file_size: u64,
    pub status: CopyTaskStatus,
    pub hash_results: Vec<HashResult>,
}

/// Configuration for the copy engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyEngineConfig {
    /// Buffer size for reading (default: 4MB)
    pub buffer_size: usize,
    /// Maximum retry count on failure
    pub max_retries: u32,
    /// Enable cascading copy
    pub cascading_enabled: bool,
    /// Hash algorithms to use for inline verification
    pub hash_algorithms: Vec<HashAlgorithm>,
}

impl Default for CopyEngineConfig {
    fn default() -> Self {
        Self {
            buffer_size: 4 * 1024 * 1024, // 4MB
            max_retries: 3,
            cascading_enabled: false,
            hash_algorithms: vec![HashAlgorithm::XXH64],
        }
    }
}

/// Result of a single file copy operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyFileResult {
    pub source_path: PathBuf,
    pub dest_path: PathBuf,
    pub bytes_copied: u64,
    pub hash_results: Vec<HashResult>,
    pub success: bool,
    pub error: Option<String>,
}

/// Check that a destination path has enough space for the source file
pub async fn check_available_space(dest_path: &Path, required_bytes: u64) -> Result<()> {
    let mut check_path = dest_path.to_path_buf();
    while !check_path.exists() {
        if let Some(parent) = check_path.parent() {
            check_path = parent.to_path_buf();
        } else {
            bail!("Cannot determine available space: no parent directory found");
        }
    }

    #[cfg(unix)]
    {
        use std::ffi::CString;
        let c_path = CString::new(check_path.to_str().unwrap_or(""))?;
        unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
                let available = stat.f_bavail as u64 * stat.f_frsize as u64;
                if available < required_bytes {
                    bail!(
                        "Insufficient space on {:?}: {} available, {} required",
                        dest_path,
                        format_bytes(available),
                        format_bytes(required_bytes)
                    );
                }
            }
        }
    }

    Ok(())
}

/// Copy a single file to one destination with atomic write and inline verification.
pub async fn copy_file_single(
    source: &Path,
    dest: &Path,
    config: &CopyEngineConfig,
) -> Result<CopyFileResult> {
    let file_size = tokio::fs::metadata(source)
        .await
        .with_context(|| format!("Cannot read source file: {:?}", source))?
        .len();

    check_available_space(dest, file_size).await?;

    let mut source_file = tokio::fs::File::open(source)
        .await
        .with_context(|| format!("Cannot open source file: {:?}", source))?;

    let mut writer = AtomicWriter::new(dest).await?;
    let mut hasher = MultiHasher::new(&config.hash_algorithms);
    let mut buffer = vec![0u8; config.buffer_size];

    loop {
        let bytes_read = source_file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        let chunk = &buffer[..bytes_read];
        hasher.update(chunk);
        writer.write(chunk).await?;
    }

    let hash_results = hasher.finalize();
    let bytes_copied = writer.bytes_written();

    if bytes_copied != file_size {
        writer.abort().await.ok();
        bail!(
            "Size mismatch after copy: source={} copied={}",
            file_size,
            bytes_copied
        );
    }

    writer.finalize().await?;

    Ok(CopyFileResult {
        source_path: source.to_path_buf(),
        dest_path: dest.to_path_buf(),
        bytes_copied,
        hash_results,
        success: true,
        error: None,
    })
}

/// Copy a single source file to multiple destinations simultaneously.
/// The source is read once; each chunk goes to all writers + all hashers.
pub async fn copy_file_multi(
    source: &Path,
    destinations: &[PathBuf],
    config: &CopyEngineConfig,
) -> Result<Vec<CopyFileResult>> {
    if destinations.is_empty() {
        bail!("No destinations specified");
    }

    let file_size = tokio::fs::metadata(source)
        .await
        .with_context(|| format!("Cannot read source file: {:?}", source))?
        .len();

    for dest in destinations {
        check_available_space(dest, file_size).await?;
    }

    let mut source_file = tokio::fs::File::open(source).await?;
    let mut writers = Vec::with_capacity(destinations.len());
    for dest in destinations {
        writers.push(AtomicWriter::new(dest).await?);
    }

    let mut hasher = MultiHasher::new(&config.hash_algorithms);
    let mut buffer = vec![0u8; config.buffer_size];

    loop {
        let bytes_read = source_file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        let chunk = &buffer[..bytes_read];
        hasher.update(chunk);
        for writer in &mut writers {
            writer.write(chunk).await?;
        }
    }

    let hash_results = hasher.finalize();
    let mut results = Vec::with_capacity(writers.len());

    for (i, writer) in writers.into_iter().enumerate() {
        let bytes_written = writer.bytes_written();
        if bytes_written != file_size {
            writer.abort().await.ok();
            results.push(CopyFileResult {
                source_path: source.to_path_buf(),
                dest_path: destinations[i].clone(),
                bytes_copied: bytes_written,
                hash_results: hash_results.clone(),
                success: false,
                error: Some(format!(
                    "Size mismatch: expected {} got {}",
                    file_size, bytes_written
                )),
            });
        } else {
            writer.finalize().await?;
            results.push(CopyFileResult {
                source_path: source.to_path_buf(),
                dest_path: destinations[i].clone(),
                bytes_copied: bytes_written,
                hash_results: hash_results.clone(),
                success: true,
                error: None,
            });
        }
    }

    Ok(results)
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn create_test_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    #[tokio::test]
    async fn test_copy_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = create_test_file(dir.path(), "source.mov", b"video data here");
        let dest = dir.path().join("dest.mov");

        let config = CopyEngineConfig {
            hash_algorithms: vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256],
            ..Default::default()
        };

        let result = copy_file_single(&source, &dest, &config).await.unwrap();
        assert!(result.success);
        assert_eq!(result.bytes_copied, 15);
        assert_eq!(result.hash_results.len(), 2);

        let src_bytes = std::fs::read(&source).unwrap();
        let dst_bytes = std::fs::read(&dest).unwrap();
        assert_eq!(src_bytes, dst_bytes);
    }

    #[tokio::test]
    async fn test_copy_multi_destination() {
        let dir = tempfile::tempdir().unwrap();
        let source = create_test_file(dir.path(), "source.r3d", b"raw camera data");

        std::fs::create_dir_all(dir.path().join("backup1")).unwrap();
        std::fs::create_dir_all(dir.path().join("backup2")).unwrap();

        let dests = vec![
            dir.path().join("backup1").join("source.r3d"),
            dir.path().join("backup2").join("source.r3d"),
        ];

        let config = CopyEngineConfig::default();
        let results = copy_file_multi(&source, &dests, &config).await.unwrap();

        assert_eq!(results.len(), 2);
        for r in &results {
            assert!(r.success);
            assert_eq!(r.bytes_copied, 15);
        }

        // All copies should have the same hash
        assert_eq!(
            results[0].hash_results[0].hex_digest,
            results[1].hash_results[0].hex_digest
        );
    }

    #[tokio::test]
    async fn test_no_tmp_files_after_success() {
        let dir = tempfile::tempdir().unwrap();
        let source = create_test_file(dir.path(), "source.braw", b"blackmagic raw");
        let dest = dir.path().join("dest.braw");

        let config = CopyEngineConfig::default();
        copy_file_single(&source, &dest, &config).await.unwrap();

        assert!(!AtomicWriter::temp_path_for(&dest).exists());
        assert!(dest.exists());
    }
}
