//! Copy Engine — Multi-path parallel copy with single-source-read optimization.
//!
//! Core responsibilities:
//! - Read source file once, write to multiple destinations simultaneously
//! - Cascading copy (fast device first, then slow devices)
//! - Inline hash verification during copy
//! - Atomic write (.tmp + rename)
//! - Pre-copy space validation
//! - File conflict detection (skip existing files)
//! - Chunk-level pause/cancel support
//! - Intra-file progress reporting

pub mod atomic_writer;

use crate::hash_engine::{HashAlgorithm, HashResult, MultiHasher};
use anyhow::{bail, Context, Result};
use atomic_writer::AtomicWriter;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Policy for handling existing destination files
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub enum FileConflictPolicy {
    /// Overwrite without checking (used for retries of known failures)
    Overwrite,
    /// Skip if destination exists AND content is verified identical via XXH3 hash.
    /// File size alone is NEVER used as proof of data integrity.
    /// Process: size differs → overwrite; size matches → hash both files → compare.
    #[default]
    SkipIfVerified,
    /// Skip if destination exists (regardless of content)
    SkipAlways,
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
    /// Policy for handling existing destination files
    #[serde(default)]
    pub conflict_policy: FileConflictPolicy,
}

impl Default for CopyEngineConfig {
    fn default() -> Self {
        Self {
            buffer_size: 4 * 1024 * 1024, // 4MB
            max_retries: 3,
            cascading_enabled: false,
            hash_algorithms: vec![HashAlgorithm::XXH64],
            conflict_policy: FileConflictPolicy::SkipIfVerified,
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
    /// Whether this file was skipped due to conflict policy
    #[serde(default)]
    pub skipped: bool,
    pub error: Option<String>,
}

/// Runtime control for pause/cancel and progress reporting during copy.
/// Uses raw AtomicBool references to avoid circular dependency on workflow module.
pub struct CopyControl<'a> {
    /// If set and true, abort the copy immediately
    pub cancel_flag: Option<&'a AtomicBool>,
    /// If set and true, pause the copy until it becomes false
    pub pause_flag: Option<&'a AtomicBool>,
    /// Progress callback: (bytes_written_this_file, total_file_size)
    /// Called after each buffer write. Callers should throttle UI updates.
    pub on_progress: Option<Box<dyn Fn(u64, u64) + Send + Sync + 'a>>,
}

impl<'a> CopyControl<'a> {
    /// Create a no-op control (no pause, no cancel, no progress)
    pub fn none() -> Self {
        Self {
            cancel_flag: None,
            pause_flag: None,
            on_progress: None,
        }
    }
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
        let c_path = CString::new(check_path.to_string_lossy().as_ref())?;
        unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
                // f_frsize can be 0 on some APFS configurations; fall back to f_bsize
                let block_size = if stat.f_frsize > 0 {
                    stat.f_frsize
                } else {
                    stat.f_bsize
                };
                let available = stat.f_bavail as u64 * block_size;
                if available < required_bytes {
                    bail!(
                        "Insufficient space on {:?}: {} available, {} required",
                        dest_path,
                        format_bytes(available),
                        format_bytes(required_bytes)
                    );
                }
            } else {
                log::warn!("statvfs failed for {:?}, skipping space check", dest_path);
            }
        }
    }

    #[cfg(windows)]
    {
        match crate::volume::get_volume_space(&check_path) {
            Ok(space) => {
                if space.available_bytes < required_bytes {
                    bail!(
                        "Insufficient space on {:?}: {} available, {} required",
                        dest_path,
                        format_bytes(space.available_bytes),
                        format_bytes(required_bytes)
                    );
                }
            }
            Err(e) => {
                log::warn!(
                    "GetDiskFreeSpaceEx failed for {:?}: {}, skipping space check",
                    dest_path,
                    e
                );
            }
        }
    }

    Ok(())
}

/// Check cancel/pause flags at chunk granularity.
/// If cancelled, bails immediately. If paused, spins until resumed.
/// Note: .tmp file cleanup is handled by recovery process (`cleanup_tmp_files`).
async fn check_cancel_pause(
    cancel_flag: Option<&AtomicBool>,
    pause_flag: Option<&AtomicBool>,
) -> Result<()> {
    // Check cancel first
    if let Some(c) = cancel_flag {
        if c.load(Ordering::SeqCst) {
            bail!("Offload cancelled by user");
        }
    }
    // Check pause — spin until unpaused, checking cancel every 200ms
    if let Some(p) = pause_flag {
        while p.load(Ordering::SeqCst) {
            if let Some(c) = cancel_flag {
                if c.load(Ordering::SeqCst) {
                    bail!("Offload cancelled by user");
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    Ok(())
}

/// Compute XXH3-128 hash of a file for content verification.
/// Uses XXH3 (fastest available algorithm, ~12+ GB/s) for skip-check purposes.
async fn quick_content_hash(path: &Path, buffer_size: usize) -> Result<u128> {
    use xxhash_rust::xxh3::Xxh3;
    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("Cannot open file for hash verification: {:?}", path))?;
    let mut hasher = Xxh3::new();
    let mut buffer = vec![0u8; buffer_size];
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.digest128())
}

/// Check if a single destination file should be skipped based on conflict policy.
///
/// For `SkipIfVerified`: file size is NEVER used as proof of data integrity.
/// If the destination exists with matching size, XXH3-128 hashes of BOTH source
/// and destination are computed and compared. Only skips if hashes are identical.
///
/// `source_hash` allows pre-computed source hash reuse across multiple destinations.
/// Returns `(should_skip, source_hash_for_reuse)`.
async fn should_skip_conflict(
    source: &Path,
    dest: &Path,
    source_size: u64,
    policy: FileConflictPolicy,
    buffer_size: usize,
    source_hash: Option<u128>,
) -> (bool, Option<u128>) {
    if policy == FileConflictPolicy::Overwrite {
        return (false, source_hash);
    }
    if let Ok(meta) = tokio::fs::metadata(dest).await {
        match policy {
            FileConflictPolicy::SkipAlways => (true, source_hash),
            FileConflictPolicy::SkipIfVerified => {
                // Fast pre-filter: if sizes differ, files are definitely different → overwrite
                if meta.len() != source_size {
                    return (false, source_hash);
                }
                // Size matches — MUST verify with hash (size alone is never trusted)
                log::info!(
                    "SkipIfVerified: size match ({} bytes), hashing source: {:?}",
                    source_size,
                    source
                );
                let src_hash = match source_hash {
                    Some(h) => h,
                    None => match quick_content_hash(source, buffer_size).await {
                        Ok(h) => h,
                        Err(_) => return (false, None),
                    },
                };
                log::info!("SkipIfVerified: hashing dest: {:?}", dest);
                match quick_content_hash(dest, buffer_size).await {
                    Ok(dest_hash) => {
                        let identical = src_hash == dest_hash;
                        if identical {
                            log::info!(
                                "SkipIfVerified: identical content, skipping {:?}",
                                source.file_name().unwrap_or_default()
                            );
                        }
                        (identical, Some(src_hash))
                    }
                    Err(_) => (false, Some(src_hash)),
                }
            }
            FileConflictPolicy::Overwrite => (false, source_hash),
        }
    } else {
        (false, source_hash) // destination doesn't exist, proceed with copy
    }
}

/// Copy a single file to one destination with atomic write and inline verification.
/// Supports chunk-level pause/cancel and intra-file progress reporting.
pub async fn copy_file_single(
    source: &Path,
    dest: &Path,
    config: &CopyEngineConfig,
    control: &CopyControl<'_>,
) -> Result<CopyFileResult> {
    let file_size = tokio::fs::metadata(source)
        .await
        .with_context(|| format!("Cannot read source file: {:?}", source))?
        .len();

    // Check conflict policy (hash-based verification for SkipIfVerified)
    let (should_skip, _) = should_skip_conflict(
        source,
        dest,
        file_size,
        config.conflict_policy,
        config.buffer_size,
        None,
    )
    .await;
    if should_skip {
        return Ok(CopyFileResult {
            source_path: source.to_path_buf(),
            dest_path: dest.to_path_buf(),
            bytes_copied: 0,
            hash_results: vec![],
            success: true,
            skipped: true,
            error: None,
        });
    }

    check_available_space(dest, file_size).await?;

    let mut source_file = tokio::fs::File::open(source)
        .await
        .with_context(|| format!("Cannot open source file: {:?}", source))?;

    let mut writer = AtomicWriter::new(dest).await?;
    let mut hasher = MultiHasher::new(&config.hash_algorithms);
    let mut buffer = vec![0u8; config.buffer_size];

    loop {
        // Check cancel/pause before each chunk read
        check_cancel_pause(control.cancel_flag, control.pause_flag).await?;

        let bytes_read = source_file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        let chunk = &buffer[..bytes_read];
        hasher.update(chunk);
        writer.write(chunk).await?;

        // Report progress after each write
        if let Some(ref on_progress) = control.on_progress {
            on_progress(writer.bytes_written(), file_size);
        }
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
        skipped: false,
        error: None,
    })
}

/// Copy a single source file to multiple destinations simultaneously.
/// The source is read once; each chunk goes to all writers + all hashers.
/// Supports chunk-level pause/cancel and intra-file progress reporting.
pub async fn copy_file_multi(
    source: &Path,
    destinations: &[PathBuf],
    config: &CopyEngineConfig,
    control: &CopyControl<'_>,
) -> Result<Vec<CopyFileResult>> {
    if destinations.is_empty() {
        bail!("No destinations specified");
    }

    let file_size = tokio::fs::metadata(source)
        .await
        .with_context(|| format!("Cannot read source file: {:?}", source))?
        .len();

    // Check conflict policy for each destination — track which ones to skip.
    // Source hash is computed once and reused across all destinations.
    let mut skip_flags = Vec::with_capacity(destinations.len());
    let mut any_needs_copy = false;
    let mut cached_source_hash: Option<u128> = None;
    for dest in destinations {
        let (skip, src_hash) = should_skip_conflict(
            source,
            dest,
            file_size,
            config.conflict_policy,
            config.buffer_size,
            cached_source_hash,
        )
        .await;
        cached_source_hash = src_hash;
        if !skip {
            any_needs_copy = true;
        }
        skip_flags.push(skip);
    }

    // If ALL destinations are skipped, return immediately
    if !any_needs_copy {
        return Ok(destinations
            .iter()
            .map(|dest| CopyFileResult {
                source_path: source.to_path_buf(),
                dest_path: dest.clone(),
                bytes_copied: 0,
                hash_results: vec![],
                success: true,
                skipped: true,
                error: None,
            })
            .collect());
    }

    // Check space only for non-skipped destinations
    for (i, dest) in destinations.iter().enumerate() {
        if !skip_flags[i] {
            check_available_space(dest, file_size).await?;
        }
    }

    let mut source_file = tokio::fs::File::open(source).await?;

    // Create writers only for non-skipped destinations
    let mut writers: Vec<Option<AtomicWriter>> = Vec::with_capacity(destinations.len());
    for (i, dest) in destinations.iter().enumerate() {
        if skip_flags[i] {
            writers.push(None);
        } else {
            writers.push(Some(AtomicWriter::new(dest).await?));
        }
    }

    let mut hasher = MultiHasher::new(&config.hash_algorithms);
    let mut buffer = vec![0u8; config.buffer_size];
    let mut total_written: u64 = 0;
    // Per-writer error tracking: if a destination write fails, skip it on subsequent chunks
    let mut writer_errors: Vec<Option<String>> = vec![None; destinations.len()];

    loop {
        // Check cancel/pause before each chunk read
        check_cancel_pause(control.cancel_flag, control.pause_flag).await?;

        let bytes_read = source_file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        let chunk = &buffer[..bytes_read];
        hasher.update(chunk);
        for (idx, writer_opt) in writers.iter_mut().enumerate() {
            if writer_errors[idx].is_some() {
                continue; // already failed, skip
            }
            if let Some(ref mut writer) = writer_opt {
                if let Err(e) = writer.write(chunk).await {
                    writer_errors[idx] = Some(e.to_string());
                    // Take ownership for abort (abort consumes self)
                    if let Some(w) = writer_opt.take() {
                        w.abort().await.ok();
                    }
                }
            }
        }
        total_written += bytes_read as u64;

        // Report progress after each write
        if let Some(ref on_progress) = control.on_progress {
            on_progress(total_written, file_size);
        }
    }

    let hash_results = hasher.finalize();
    let mut results = Vec::with_capacity(writers.len());

    for (i, writer_opt) in writers.into_iter().enumerate() {
        if skip_flags[i] {
            results.push(CopyFileResult {
                source_path: source.to_path_buf(),
                dest_path: destinations[i].clone(),
                bytes_copied: 0,
                hash_results: vec![],
                success: true,
                skipped: true,
                error: None,
            });
            continue;
        }
        // Check if this writer failed during copy
        if let Some(err_msg) = &writer_errors[i] {
            results.push(CopyFileResult {
                source_path: source.to_path_buf(),
                dest_path: destinations[i].clone(),
                bytes_copied: 0,
                hash_results: vec![],
                success: false,
                skipped: false,
                error: Some(err_msg.clone()),
            });
            continue;
        }
        let writer = writer_opt.expect("writer should exist for non-skipped destination");
        let bytes_written = writer.bytes_written();
        if bytes_written != file_size {
            writer.abort().await.ok();
            results.push(CopyFileResult {
                source_path: source.to_path_buf(),
                dest_path: destinations[i].clone(),
                bytes_copied: bytes_written,
                hash_results: hash_results.clone(),
                success: false,
                skipped: false,
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
                skipped: false,
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

        let result = copy_file_single(&source, &dest, &config, &CopyControl::none())
            .await
            .unwrap();
        assert!(result.success);
        assert!(!result.skipped);
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
        let results = copy_file_multi(&source, &dests, &config, &CopyControl::none())
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        for r in &results {
            assert!(r.success);
            assert!(!r.skipped);
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
        copy_file_single(&source, &dest, &config, &CopyControl::none())
            .await
            .unwrap();

        assert!(!AtomicWriter::temp_path_for(&dest).exists());
        assert!(dest.exists());
    }

    #[tokio::test]
    async fn test_skip_verified_identical_content() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"video data here";
        let source = create_test_file(dir.path(), "source.mov", content);
        let dest = dir.path().join("dest.mov");
        // Pre-create dest with identical content (same size AND same hash)
        {
            let mut f = std::fs::File::create(&dest).unwrap();
            f.write_all(content).unwrap();
        }

        let config = CopyEngineConfig {
            conflict_policy: FileConflictPolicy::SkipIfVerified,
            ..Default::default()
        };
        let result = copy_file_single(&source, &dest, &config, &CopyControl::none())
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.skipped); // identical content → skip
        assert_eq!(result.bytes_copied, 0);
    }

    #[tokio::test]
    async fn test_overwrite_same_size_different_content() {
        let dir = tempfile::tempdir().unwrap();
        let source = create_test_file(dir.path(), "source.mov", b"correct data!!!");
        let dest = dir.path().join("dest.mov");
        // Pre-create dest with SAME SIZE but DIFFERENT content (simulates corruption)
        {
            let mut f = std::fs::File::create(&dest).unwrap();
            f.write_all(b"corrupt data!!!").unwrap();
        }

        let config = CopyEngineConfig {
            conflict_policy: FileConflictPolicy::SkipIfVerified,
            ..Default::default()
        };
        let result = copy_file_single(&source, &dest, &config, &CopyControl::none())
            .await
            .unwrap();
        assert!(result.success);
        assert!(!result.skipped); // same size but different hash → overwrite
        assert_eq!(result.bytes_copied, 15);
        // Verify dest now has correct content
        let dest_content = std::fs::read(&dest).unwrap();
        assert_eq!(dest_content, b"correct data!!!");
    }

    #[tokio::test]
    async fn test_overwrite_existing_different_size() {
        let dir = tempfile::tempdir().unwrap();
        let source = create_test_file(dir.path(), "source.mov", b"new video data here!");
        let dest = dir.path().join("dest.mov");
        // Pre-create dest with different size
        {
            let mut f = std::fs::File::create(&dest).unwrap();
            f.write_all(b"old").unwrap();
        }

        let config = CopyEngineConfig {
            conflict_policy: FileConflictPolicy::SkipIfVerified,
            ..Default::default()
        };
        let result = copy_file_single(&source, &dest, &config, &CopyControl::none())
            .await
            .unwrap();
        assert!(result.success);
        assert!(!result.skipped);
        assert_eq!(result.bytes_copied, 20);
    }

    #[tokio::test]
    async fn test_skip_always_policy() {
        let dir = tempfile::tempdir().unwrap();
        let source = create_test_file(dir.path(), "source.mov", b"new data");
        let dest = dir.path().join("dest.mov");
        // Pre-create dest with different size
        {
            let mut f = std::fs::File::create(&dest).unwrap();
            f.write_all(b"old").unwrap();
        }

        let config = CopyEngineConfig {
            conflict_policy: FileConflictPolicy::SkipAlways,
            ..Default::default()
        };
        let result = copy_file_single(&source, &dest, &config, &CopyControl::none())
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.skipped);
    }

    #[tokio::test]
    async fn test_multi_partial_skip_verified() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"raw camera data";
        let source = create_test_file(dir.path(), "source.r3d", content);

        std::fs::create_dir_all(dir.path().join("backup1")).unwrap();
        std::fs::create_dir_all(dir.path().join("backup2")).unwrap();

        // Pre-create backup1 with identical content (will be hash-verified and skipped)
        {
            let dest1 = dir.path().join("backup1").join("source.r3d");
            let mut f = std::fs::File::create(&dest1).unwrap();
            f.write_all(content).unwrap();
        }

        let dests = vec![
            dir.path().join("backup1").join("source.r3d"),
            dir.path().join("backup2").join("source.r3d"),
        ];

        let config = CopyEngineConfig {
            conflict_policy: FileConflictPolicy::SkipIfVerified,
            ..Default::default()
        };
        let results = copy_file_multi(&source, &dests, &config, &CopyControl::none())
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results[0].skipped); // backup1: identical hash → skipped
        assert!(!results[1].skipped); // backup2: doesn't exist → copied
        assert!(results[1].success);
        assert_eq!(results[1].bytes_copied, 15);
    }

    #[tokio::test]
    async fn test_multi_skip_corrupt_same_size() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"raw camera data";
        let source = create_test_file(dir.path(), "source.r3d", content);

        std::fs::create_dir_all(dir.path().join("backup1")).unwrap();
        std::fs::create_dir_all(dir.path().join("backup2")).unwrap();

        // Pre-create backup1 with SAME SIZE but DIFFERENT content (corrupt file)
        {
            let dest1 = dir.path().join("backup1").join("source.r3d");
            let mut f = std::fs::File::create(&dest1).unwrap();
            f.write_all(b"corrupt data!!!").unwrap(); // same 15 bytes, different content
        }

        let dests = vec![
            dir.path().join("backup1").join("source.r3d"),
            dir.path().join("backup2").join("source.r3d"),
        ];

        let config = CopyEngineConfig {
            conflict_policy: FileConflictPolicy::SkipIfVerified,
            ..Default::default()
        };
        let results = copy_file_multi(&source, &dests, &config, &CopyControl::none())
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(!results[0].skipped); // backup1: hash mismatch → overwritten
        assert!(results[0].success);
        assert!(!results[1].skipped); // backup2: doesn't exist → copied
        assert!(results[1].success);
        // Verify backup1 now has correct content
        let dest1_content = std::fs::read(dir.path().join("backup1").join("source.r3d")).unwrap();
        assert_eq!(dest1_content, content);
    }

    #[tokio::test]
    async fn test_cancel_during_copy() {
        let dir = tempfile::tempdir().unwrap();
        // Create a larger file to ensure we get at least one iteration
        let data = vec![0xABu8; 1024 * 1024]; // 1MB
        let source = dir.path().join("large_source.mov");
        std::fs::write(&source, &data).unwrap();
        let dest = dir.path().join("large_dest.mov");

        let cancel = AtomicBool::new(true); // pre-cancelled
        let config = CopyEngineConfig {
            buffer_size: 4096, // small buffer to test cancel between chunks
            ..Default::default()
        };
        let control = CopyControl {
            cancel_flag: Some(&cancel),
            pause_flag: None,
            on_progress: None,
        };

        let result = copy_file_single(&source, &dest, &config, &control).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("cancelled by user"));
    }

    #[tokio::test]
    async fn test_pause_resume_continues_inflight_temp_file() {
        use std::sync::atomic::AtomicU64;
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let data = vec![0xCDu8; 512 * 1024];
        let source = dir.path().join("pause_source.mov");
        std::fs::write(&source, &data).unwrap();
        let dest = dir.path().join("pause_dest.mov");
        let tmp = AtomicWriter::temp_path_for(&dest);

        let pause = AtomicBool::new(false);
        let pause_triggered = AtomicBool::new(false);
        let progress = Arc::new(AtomicU64::new(0));
        let progress_for_cb = progress.clone();
        let pause_for_cb = &pause;
        let pause_triggered_for_cb = &pause_triggered;

        let config = CopyEngineConfig {
            buffer_size: 4096,
            ..Default::default()
        };
        let control = CopyControl {
            cancel_flag: None,
            pause_flag: Some(&pause),
            on_progress: Some(Box::new(move |written, _total| {
                progress_for_cb.store(written, Ordering::SeqCst);
                if written >= 16 * 1024 && !pause_triggered_for_cb.swap(true, Ordering::SeqCst) {
                    pause_for_cb.store(true, Ordering::SeqCst);
                }
            })),
        };

        let copy_future = copy_file_single(&source, &dest, &config, &control);
        tokio::pin!(copy_future);

        loop {
            tokio::select! {
                result = &mut copy_future => {
                    panic!("copy finished before pause was observed: {:?}", result);
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {
                    if pause.load(Ordering::SeqCst) && progress.load(Ordering::SeqCst) >= 16 * 1024 {
                        break;
                    }
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        assert!(
            tmp.exists(),
            "paused copy should keep the in-flight temp file"
        );
        assert!(
            !dest.exists(),
            "final file must not appear before atomic finalize"
        );

        pause.store(false, Ordering::SeqCst);
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), &mut copy_future)
            .await
            .unwrap()
            .unwrap();

        assert!(result.success);
        assert_eq!(result.bytes_copied, data.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(!tmp.exists());
    }

    #[tokio::test]
    async fn test_cancelled_partial_copy_restarts_from_zero_on_next_copy() {
        use std::sync::atomic::AtomicU64;
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let data = vec![0xEFu8; 512 * 1024];
        let source = dir.path().join("cancel_source.mov");
        std::fs::write(&source, &data).unwrap();
        let dest = dir.path().join("cancel_dest.mov");
        let tmp = AtomicWriter::temp_path_for(&dest);

        let cancel = AtomicBool::new(false);
        let progress = Arc::new(AtomicU64::new(0));
        let progress_for_cb = progress.clone();
        let cancel_for_cb = &cancel;

        let config = CopyEngineConfig {
            buffer_size: 4096,
            ..Default::default()
        };
        let control = CopyControl {
            cancel_flag: Some(&cancel),
            pause_flag: None,
            on_progress: Some(Box::new(move |written, _total| {
                progress_for_cb.store(written, Ordering::SeqCst);
                if written >= 16 * 1024 {
                    cancel_for_cb.store(true, Ordering::SeqCst);
                }
            })),
        };

        let result = copy_file_single(&source, &dest, &config, &control).await;
        assert!(result.is_err());
        assert!(progress.load(Ordering::SeqCst) >= 16 * 1024);
        assert!(
            !tmp.exists(),
            "cancel cleanup should remove the partial temp file"
        );
        assert!(
            !dest.exists(),
            "cancelled copy should not leave a final partial file"
        );

        let resumed = copy_file_single(&source, &dest, &config, &CopyControl::none())
            .await
            .unwrap();
        assert!(resumed.success);
        assert!(!resumed.skipped);
        assert_eq!(resumed.bytes_copied, data.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[tokio::test]
    async fn test_progress_callback() {
        use std::sync::atomic::AtomicU64;
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let data = vec![0xABu8; 64 * 1024]; // 64KB
        let source = dir.path().join("progress_source.mov");
        std::fs::write(&source, &data).unwrap();
        let dest = dir.path().join("progress_dest.mov");

        let progress_count = Arc::new(AtomicU64::new(0));
        let last_bytes = Arc::new(AtomicU64::new(0));
        let pc = progress_count.clone();
        let lb = last_bytes.clone();

        let config = CopyEngineConfig {
            buffer_size: 4096, // small buffer = many progress callbacks
            ..Default::default()
        };
        let control = CopyControl {
            cancel_flag: None,
            pause_flag: None,
            on_progress: Some(Box::new(move |written, total| {
                pc.fetch_add(1, Ordering::SeqCst);
                lb.store(written, Ordering::SeqCst);
                assert_eq!(total, 64 * 1024);
            })),
        };

        let result = copy_file_single(&source, &dest, &config, &control)
            .await
            .unwrap();
        assert!(result.success);
        assert!(progress_count.load(Ordering::SeqCst) > 1); // multiple progress calls
        assert_eq!(last_bytes.load(Ordering::SeqCst), 64 * 1024); // final = total
    }
}
