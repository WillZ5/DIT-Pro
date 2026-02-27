//! Offload Workflow — End-to-end card offload pipeline orchestrator.
//!
//! Coordinates all modules into a complete offload process:
//!
//! ```text
//! ┌─────────┐    ┌──────────────┐    ┌────────┐    ┌────────┐    ┌──────┐
//! │PreFlight│ →  │SourceVerify  │ →  │  Copy  │ →  │Verify  │ →  │ Seal │
//! │(space,  │    │(hash source  │    │(read 1 │    │(re-read│    │(MHL  │
//! │ devices)│    │ files first) │    │write N)│    │ check) │    │gen)  │
//! └─────────┘    └──────────────┘    └────────┘    └────────┘    └──────┘
//! ```
//!
//! ## Features
//! - **Read-once-write-many**: Source read once, written to all destinations simultaneously
//! - **Inline verification**: Hashes computed during copy, no second read for basic check
//! - **Post-copy verification**: Optional re-read of destinations for paranoid verification
//! - **Crash recovery**: SQLite checkpoint ensures safe resume after any interruption
//! - **ASC MHL sealing**: Chain-of-custody manifests generated after successful copy
//! - **Event-driven progress**: Real-time events for frontend display

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::checkpoint::{self, STATUS_COPYING};
use crate::copy_engine::{self, CopyEngineConfig};
use crate::hash_engine::{self, HashAlgorithm, HashEngineConfig, HashResult};
use crate::mhl::{self, MhlConfig, MhlProcessType};
use crate::volume;

// ─── Configuration ───────────────────────────────────────────────────────

/// User's decision for a conflicting file
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictAction {
    Skip,
    Overwrite,
    KeepBoth,
}

/// A single conflict resolution from the user
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictResolution {
    pub rel_path: String,
    pub dest_path: String,
    pub action: ConflictAction,
}

/// Configuration for an offload workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffloadConfig {
    pub job_id: String,
    pub job_name: String,
    pub source_path: PathBuf,
    pub dest_paths: Vec<PathBuf>,
    pub hash_algorithms: Vec<HashAlgorithm>,
    pub buffer_size: usize,
    /// Hash source files before copy to establish baseline
    pub source_verify: bool,
    /// Re-read destination files after copy and compare hashes
    pub post_verify: bool,
    /// Generate ASC MHL manifests after successful copy
    pub generate_mhl: bool,
    pub max_retries: u32,
    /// Cascading copy: copy to first (fastest) dest first, then cascade from
    /// that copy to remaining (slower) destinations. Frees source card sooner.
    #[serde(default)]
    pub cascade: bool,
    /// Optional per-file conflict resolutions from duplicate detection dialog.
    /// Key: "rel_path::dest_path" → action
    #[serde(default)]
    pub conflict_resolutions: HashMap<String, ConflictAction>,
}

impl Default for OffloadConfig {
    fn default() -> Self {
        Self {
            job_id: uuid::Uuid::new_v4().to_string(),
            job_name: "Offload".to_string(),
            source_path: PathBuf::new(),
            dest_paths: Vec::new(),
            hash_algorithms: vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256],
            buffer_size: 4 * 1024 * 1024,
            source_verify: true,
            post_verify: true,
            generate_mhl: true,
            max_retries: 3,
            cascade: false,
            conflict_resolutions: HashMap::new(),
        }
    }
}

// ─── Phase & Event Types ─────────────────────────────────────────────────

/// Current phase of the offload workflow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OffloadPhase {
    PreFlight,
    SourceVerify,
    Copying,
    /// Cascading: copying from primary (fast) dest → secondary (slower) dests
    Cascading,
    Verifying,
    Sealing,
    Complete,
    Failed,
}

impl std::fmt::Display for OffloadPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreFlight => write!(f, "Pre-Flight Check"),
            Self::SourceVerify => write!(f, "Source Verification"),
            Self::Copying => write!(f, "Copying"),
            Self::Cascading => write!(f, "Cascading to Secondary Destinations"),
            Self::Verifying => write!(f, "Post-Copy Verification"),
            Self::Sealing => write!(f, "MHL Sealing"),
            Self::Complete => write!(f, "Complete"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

/// Events emitted during the offload workflow for frontend updates
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OffloadEvent {
    PhaseChanged {
        phase: OffloadPhase,
        message: String,
    },
    SourceHashCompleted {
        rel_path: String,
        hashes: Vec<HashResult>,
        file_index: usize,
        total_files: usize,
    },
    FileCopyStarted {
        rel_path: String,
        file_size: u64,
        dest_count: usize,
    },
    FileCopyCompleted {
        rel_path: String,
        file_size: u64,
        hashes: Vec<HashResult>,
        file_index: usize,
        total_files: usize,
    },
    FileVerified {
        rel_path: String,
        dest_path: String,
        verified: bool,
        mismatch_detail: Option<String>,
    },
    JobProgress {
        completed_files: usize,
        total_files: usize,
        completed_bytes: u64,
        total_bytes: u64,
        phase: OffloadPhase,
        elapsed_secs: f64,
        /// Optional per-file status message (e.g. "Copying (3/142) file.mxf → A001")
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Warning {
        message: String,
    },
    Complete {
        total_files: usize,
        total_bytes: u64,
        duration_secs: f64,
        mhl_paths: Vec<String>,
    },
    Error {
        message: String,
    },
    FileSkipped {
        rel_path: String,
        reason: String,
    },
    DuplicateConflict {
        rel_path: String,
        source_hash: String,
        dest_hash: String,
    },
    Paused,
    Resumed,
    Terminated,
}

// ─── Result ──────────────────────────────────────────────────────────────

/// Final result of the offload workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffloadResult {
    pub job_id: String,
    pub success: bool,
    pub total_files: usize,
    pub total_bytes: u64,
    pub failed_files: usize,
    pub duration_secs: f64,
    pub source_hashes: HashMap<String, Vec<HashResult>>,
    pub mhl_paths: Vec<PathBuf>,
    pub errors: Vec<String>,
}

// ─── Source File Info ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct SourceFile {
    rel_path: String,
    abs_path: PathBuf,
    size: u64,
}

// ─── Helpers ────────────────────────────────────────────────────────────

/// Generate a "keep both" path by appending _copy (or _copy_2, _copy_3, etc.)
/// Example: photo.mov → photo_copy.mov → photo_copy_2.mov
fn generate_keep_both_path(dest: &PathBuf) -> PathBuf {
    let stem = dest
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let ext = dest
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = dest.parent().unwrap_or(Path::new(""));

    // Try _copy first, then _copy_2, _copy_3, etc.
    let candidate = parent.join(format!("{}_copy{}", stem, ext));
    if !candidate.exists() {
        return candidate;
    }

    for n in 2..=999 {
        let candidate = parent.join(format!("{}_copy_{}{}", stem, n, ext));
        if !candidate.exists() {
            return candidate;
        }
    }

    // Fallback: use timestamp
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    parent.join(format!("{}_{}{}", stem, ts, ext))
}

// ─── Workflow Orchestrator ───────────────────────────────────────────────

/// Cancellation token for stopping a running workflow.
/// Clone this and call `cancel()` to request cancellation.
#[derive(Clone)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Request cancellation of the workflow.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Check if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Get a reference to the inner AtomicBool (for passing to copy_engine).
    pub fn as_atomic(&self) -> &AtomicBool {
        &self.0
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Pause token for suspending/resuming a running workflow.
/// Clone this and call `pause()` / `resume()` to control suspension.
pub struct PauseToken(Arc<AtomicBool>);

impl PauseToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn pause(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn resume(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
    pub fn is_paused(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub fn clone_token(&self) -> Self {
        Self(Arc::clone(&self.0))
    }

    /// Get a reference to the inner AtomicBool (for passing to copy_engine).
    pub fn as_atomic(&self) -> &AtomicBool {
        &self.0
    }
}

impl Default for PauseToken {
    fn default() -> Self {
        Self::new()
    }
}

/// The offload workflow orchestrator.
///
/// Ties together: space check → source hash → copy → verify → MHL seal
pub struct OffloadWorkflow {
    config: OffloadConfig,
    db: Arc<std::sync::Mutex<Connection>>,
    event_tx: mpsc::UnboundedSender<OffloadEvent>,
    cancel: CancelToken,
    pause: PauseToken,
}

impl OffloadWorkflow {
    pub fn new(
        config: OffloadConfig,
        db: Arc<std::sync::Mutex<Connection>>,
        event_tx: mpsc::UnboundedSender<OffloadEvent>,
    ) -> Self {
        Self {
            config,
            db,
            event_tx,
            cancel: CancelToken::new(),
            pause: PauseToken::new(),
        }
    }

    pub fn with_cancel(
        config: OffloadConfig,
        db: Arc<std::sync::Mutex<Connection>>,
        event_tx: mpsc::UnboundedSender<OffloadEvent>,
        cancel: CancelToken,
    ) -> Self {
        Self {
            config,
            db,
            event_tx,
            cancel,
            pause: PauseToken::new(),
        }
    }

    pub fn with_cancel_and_pause(
        config: OffloadConfig,
        db: Arc<std::sync::Mutex<Connection>>,
        event_tx: mpsc::UnboundedSender<OffloadEvent>,
        cancel: CancelToken,
        pause: PauseToken,
    ) -> Self {
        Self {
            config,
            db,
            event_tx,
            cancel,
            pause,
        }
    }

    /// Check if the workflow has been cancelled and bail if so.
    fn check_cancelled(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            bail!("Offload cancelled by user");
        }
        Ok(())
    }

    /// Wait while paused, checking for cancellation every 200ms.
    /// Emits Paused event on entry and Resumed event on exit.
    async fn check_paused(&self) -> Result<()> {
        if self.pause.is_paused() {
            self.emit(OffloadEvent::Paused);
            while self.pause.is_paused() {
                self.check_cancelled()?;
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            self.emit(OffloadEvent::Resumed);
        }
        self.check_cancelled()
    }

    /// Send an event (silently ignores if receiver dropped)
    pub fn emit(&self, event: OffloadEvent) {
        self.event_tx.send(event).ok();
    }

    /// Execute the complete offload pipeline.
    pub async fn execute(&self) -> Result<OffloadResult> {
        let start = Instant::now();
        let mut errors: Vec<String> = Vec::new();
        let mut mhl_paths: Vec<PathBuf> = Vec::new();

        // ── Phase 1: PreFlight ──────────────────────────────────────
        self.emit(OffloadEvent::PhaseChanged {
            phase: OffloadPhase::PreFlight,
            message: "Scanning source and checking destinations...".into(),
        });

        let source_files = self.scan_source().await?;
        if source_files.is_empty() {
            bail!(
                "No files found in source directory: {:?}",
                self.config.source_path
            );
        }

        let total_bytes: u64 = source_files.iter().map(|f| f.size).sum();
        let total_files = source_files.len();

        self.preflight_space_check(&source_files).await?;
        self.create_db_records(&source_files)?;

        // Mark job as actively copying
        {
            let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            conn.execute(
                "UPDATE jobs SET status = 'copying', updated_at = datetime('now') WHERE id = ?1",
                rusqlite::params![self.config.job_id],
            )?;
        }

        // ── Phase 2: Source Verification (optional) ─────────────────
        let mut source_hashes: HashMap<String, Vec<HashResult>> = HashMap::new();

        if self.config.source_verify {
            self.emit(OffloadEvent::PhaseChanged {
                phase: OffloadPhase::SourceVerify,
                message: format!("Hashing {} source files...", total_files),
            });
            source_hashes = self.hash_source_files(&source_files).await?;
        }

        // ── Phase 3: Copy ───────────────────────────────────────────
        let use_cascade = self.config.cascade && self.config.dest_paths.len() >= 2;

        if use_cascade {
            // Phase 3a: Source → Primary (fast) destination only
            self.emit(OffloadEvent::PhaseChanged {
                phase: OffloadPhase::Copying,
                message: format!(
                    "Copying {} files to primary destination (cascade mode)...",
                    total_files,
                ),
            });

            let copy_hashes_primary = self
                .copy_to_primary(&source_files, &start, total_bytes)
                .await?;

            if !self.config.source_verify {
                source_hashes = copy_hashes_primary;
            }

            // Phase 3b: Primary → Secondary destinations (source card now free)
            self.emit(OffloadEvent::PhaseChanged {
                phase: OffloadPhase::Cascading,
                message: format!(
                    "Cascading {} files to {} secondary destination(s)...",
                    total_files,
                    self.config.dest_paths.len() - 1,
                ),
            });

            self.cascade_from_primary(&source_files, &start, total_bytes)
                .await?;
        } else {
            // Standard: read source once → write all destinations simultaneously
            self.emit(OffloadEvent::PhaseChanged {
                phase: OffloadPhase::Copying,
                message: format!(
                    "Copying {} files to {} destination(s)...",
                    total_files,
                    self.config.dest_paths.len()
                ),
            });

            let copy_hashes = self
                .copy_all_files(&source_files, &start, total_bytes)
                .await?;

            if !self.config.source_verify {
                source_hashes = copy_hashes;
            }
        }

        // ── Phase 4: Post-Copy Verification (optional) ──────────────
        let mut failed_count = 0;

        if self.config.post_verify {
            {
                let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                conn.execute(
                    "UPDATE jobs SET status = 'verifying', updated_at = datetime('now') WHERE id = ?1",
                    rusqlite::params![self.config.job_id],
                )?;
            }
            self.emit(OffloadEvent::PhaseChanged {
                phase: OffloadPhase::Verifying,
                message: "Re-reading destination files for verification...".into(),
            });
            failed_count = self
                .verify_destinations(&source_files, &source_hashes, &mut errors)
                .await?;
        }

        // ── Phase 5: MHL Sealing (optional) ─────────────────────────
        if self.config.generate_mhl && failed_count == 0 {
            self.emit(OffloadEvent::PhaseChanged {
                phase: OffloadPhase::Sealing,
                message: "Generating ASC MHL manifests...".into(),
            });
            mhl_paths = self.seal_mhl(&source_files, &source_hashes).await?;
        }

        // ── Finalize ────────────────────────────────────────────────
        let duration = start.elapsed().as_secs_f64();
        let success = failed_count == 0 && errors.is_empty();

        // Update job status
        {
            let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            let status = if success {
                "completed"
            } else {
                "completed_with_errors"
            };
            conn.execute(
                "UPDATE jobs SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
                rusqlite::params![status, self.config.job_id],
            )?;
        }

        self.emit(OffloadEvent::PhaseChanged {
            phase: if success {
                OffloadPhase::Complete
            } else {
                OffloadPhase::Failed
            },
            message: if success {
                format!(
                    "Offload complete: {} files, {} in {:.1}s",
                    total_files,
                    format_bytes(total_bytes),
                    duration
                )
            } else {
                format!(
                    "Offload finished with {} error(s)",
                    errors.len() + failed_count
                )
            },
        });

        self.emit(OffloadEvent::Complete {
            total_files,
            total_bytes,
            duration_secs: duration,
            mhl_paths: mhl_paths
                .iter()
                .map(|p| p.to_string_lossy().into())
                .collect(),
        });

        Ok(OffloadResult {
            job_id: self.config.job_id.clone(),
            success,
            total_files,
            total_bytes,
            failed_files: failed_count,
            duration_secs: duration,
            source_hashes,
            mhl_paths,
            errors,
        })
    }

    /// Execute a resume workflow for an existing job with pending tasks.
    /// Skips source scan, DB record creation, and source verification.
    /// Only processes pending copy tasks.
    pub async fn execute_resume(&self) -> Result<OffloadResult> {
        let start = Instant::now();
        let mut errors: Vec<String> = Vec::new();
        let mhl_paths: Vec<PathBuf> = Vec::new();

        // Read pending tasks from DB
        let pending_tasks = {
            let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            checkpoint::get_pending_tasks(&conn, &self.config.job_id)?
        };

        if pending_tasks.is_empty() {
            bail!("No pending tasks to resume for job {}", self.config.job_id);
        }

        // Build source file list from pending tasks (deduplicate by source_path)
        let mut seen = std::collections::HashSet::new();
        let mut source_files: Vec<SourceFile> = Vec::new();
        for task in &pending_tasks {
            if seen.insert(task.source_path.clone()) {
                let abs_path = PathBuf::from(&task.source_path);
                let rel_path = abs_path
                    .strip_prefix(&self.config.source_path)
                    .unwrap_or(&abs_path)
                    .to_string_lossy()
                    .replace('\\', "/");
                source_files.push(SourceFile {
                    rel_path,
                    abs_path,
                    size: task.file_size as u64,
                });
            }
        }

        let total_bytes: u64 = source_files.iter().map(|f| f.size).sum();
        let total_files = source_files.len();

        // Emit resume start
        self.emit(OffloadEvent::PhaseChanged {
            phase: OffloadPhase::Copying,
            message: format!("Resuming: {} files remaining...", total_files),
        });

        // Copy pending files
        let copy_hashes = self
            .copy_all_files(&source_files, &start, total_bytes)
            .await?;

        // Post-copy verification (optional)
        let mut failed_count = 0;
        if self.config.post_verify && !copy_hashes.is_empty() {
            {
                let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                conn.execute(
                    "UPDATE jobs SET status = 'verifying', updated_at = datetime('now') WHERE id = ?1",
                    rusqlite::params![self.config.job_id],
                )?;
            }
            self.emit(OffloadEvent::PhaseChanged {
                phase: OffloadPhase::Verifying,
                message: "Re-reading destination files for verification...".into(),
            });
            failed_count = self
                .verify_destinations(&source_files, &copy_hashes, &mut errors)
                .await?;
        }

        // Finalize
        let duration = start.elapsed().as_secs_f64();
        let success = failed_count == 0 && errors.is_empty();

        // Check overall job status (including previously completed tasks)
        let overall_failed = {
            let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            let progress = checkpoint::get_job_progress(&conn, &self.config.job_id)?;
            progress.failed
        };

        {
            let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            let status = if success && overall_failed == 0 {
                "completed"
            } else {
                "completed_with_errors"
            };
            conn.execute(
                "UPDATE jobs SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
                rusqlite::params![status, self.config.job_id],
            )?;
        }

        self.emit(OffloadEvent::PhaseChanged {
            phase: if success {
                OffloadPhase::Complete
            } else {
                OffloadPhase::Failed
            },
            message: if success {
                format!(
                    "Resume complete: {} files, {} in {:.1}s",
                    total_files,
                    format_bytes(total_bytes),
                    duration
                )
            } else {
                format!(
                    "Resume finished with {} error(s)",
                    errors.len() + failed_count
                )
            },
        });

        self.emit(OffloadEvent::Complete {
            total_files,
            total_bytes,
            duration_secs: duration,
            mhl_paths: mhl_paths
                .iter()
                .map(|p| p.to_string_lossy().into())
                .collect(),
        });

        Ok(OffloadResult {
            job_id: self.config.job_id.clone(),
            success,
            total_files,
            total_bytes,
            failed_files: failed_count,
            duration_secs: duration,
            source_hashes: copy_hashes,
            mhl_paths,
            errors,
        })
    }

    // ── Internal: PreFlight ──────────────────────────────────────────

    /// Scan source directory recursively, respecting MHL ignore patterns.
    /// Uses `file_type()` (lstat) to avoid following symlinks and prevent infinite loops.
    async fn scan_source(&self) -> Result<Vec<SourceFile>> {
        let root = &self.config.source_path;
        let mut files = Vec::new();
        let mut stack = vec![root.clone()];

        let ignore_patterns: Vec<String> = mhl::DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();

        while let Some(dir) = stack.pop() {
            let mut entries = tokio::fs::read_dir(&dir)
                .await
                .with_context(|| format!("Cannot read directory: {:?}", dir))?;

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                // Use file_type() (lstat) instead of metadata() (stat) to avoid
                // following symlinks, which could cause infinite loops with circular links.
                let file_type = entry.file_type().await?;
                let rel_path = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");

                if mhl::should_ignore(&rel_path, &ignore_patterns) {
                    continue;
                }

                if file_type.is_symlink() {
                    // Skip symlinks to avoid circular references
                    continue;
                } else if file_type.is_dir() {
                    stack.push(path);
                } else if file_type.is_file() {
                    // Get metadata for file size (safe: we know it's a regular file)
                    let metadata = entry.metadata().await?;
                    files.push(SourceFile {
                        rel_path,
                        abs_path: path,
                        size: metadata.len(),
                    });
                }
            }
        }

        files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        Ok(files)
    }

    /// Check all destinations have enough space.
    async fn preflight_space_check(&self, files: &[SourceFile]) -> Result<()> {
        let total_size: u64 = files.iter().map(|f| f.size).sum();

        for dest in &self.config.dest_paths {
            if !dest.exists() {
                tokio::fs::create_dir_all(dest)
                    .await
                    .with_context(|| format!("Cannot create destination: {:?}", dest))?;
            }

            let space = volume::get_volume_space(dest)?;
            if space.available_bytes < total_size {
                bail!(
                    "Insufficient space on {:?}: {} available, {} required",
                    dest,
                    format_bytes(space.available_bytes),
                    format_bytes(total_size)
                );
            }
        }
        Ok(())
    }

    /// Create job + copy_task records in SQLite.
    fn create_db_records(&self, files: &[SourceFile]) -> Result<()> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;

        checkpoint::create_job(
            &conn,
            &self.config.job_id,
            &self.config.job_name,
            self.config.source_path.to_string_lossy().as_ref(),
        )?;

        for file in files {
            for dest_root in &self.config.dest_paths {
                let dest_file = dest_root.join(&file.rel_path);
                let task_id = uuid::Uuid::new_v4().to_string();
                checkpoint::insert_task(
                    &conn,
                    &task_id,
                    &self.config.job_id,
                    file.abs_path.to_string_lossy().as_ref(),
                    dest_file.to_string_lossy().as_ref(),
                    file.size,
                )?;
            }
        }
        Ok(())
    }

    // ── Internal: Source Verify ───────────────────────────────────────

    /// Hash every source file. Returns map: rel_path → hashes.
    /// Supports pause/cancel checks between files and emits per-file progress.
    async fn hash_source_files(
        &self,
        files: &[SourceFile],
    ) -> Result<HashMap<String, Vec<HashResult>>> {
        let mut map = HashMap::new();
        let cfg = HashEngineConfig {
            algorithms: self.config.hash_algorithms.clone(),
            buffer_size: self.config.buffer_size,
        };
        let total_files = files.len();
        let total_bytes: u64 = files.iter().map(|f| f.size).sum();
        let mut completed_bytes: u64 = 0;
        let hash_start = Instant::now();

        for (i, file) in files.iter().enumerate() {
            // Check pause/cancel between files
            self.check_cancelled()?;
            self.check_paused().await?;

            let file_msg = format!("Hashing ({}/{}) {}...", i + 1, total_files, file.rel_path);

            // Set up intra-file progress reporting (throttled to 100ms for responsive speed chart)
            let event_tx = self.event_tx.clone();
            let file_completed_bytes = completed_bytes;
            let progress_total_files = total_files;
            let progress_file_index = i;
            let progress_start = hash_start;
            let progress_msg = file_msg.clone();
            let last_progress_emit =
                std::sync::Mutex::new(Instant::now() - std::time::Duration::from_secs(1));
            let progress_callback = move |file_bytes_hashed: u64, _file_total: u64| {
                let mut last = last_progress_emit.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed() >= std::time::Duration::from_millis(100) {
                    let current_total = file_completed_bytes + file_bytes_hashed;
                    event_tx
                        .send(OffloadEvent::JobProgress {
                            completed_files: progress_file_index,
                            total_files: progress_total_files,
                            completed_bytes: current_total,
                            total_bytes,
                            phase: OffloadPhase::SourceVerify,
                            elapsed_secs: progress_start.elapsed().as_secs_f64(),
                            message: Some(progress_msg.clone()),
                        })
                        .ok();
                    *last = Instant::now();
                }
            };

            let hashes = hash_engine::hash_file_with_progress(
                &file.abs_path,
                &cfg,
                Some(&progress_callback),
            )
            .await
            .with_context(|| format!("Failed to hash source: {:?}", file.abs_path))?;

            completed_bytes += file.size;

            self.emit(OffloadEvent::SourceHashCompleted {
                rel_path: file.rel_path.clone(),
                hashes: hashes.clone(),
                file_index: i,
                total_files,
            });

            // Emit JobProgress after file completion
            self.emit(OffloadEvent::JobProgress {
                completed_files: i + 1,
                total_files,
                completed_bytes,
                total_bytes,
                phase: OffloadPhase::SourceVerify,
                elapsed_secs: hash_start.elapsed().as_secs_f64(),
                message: Some(file_msg),
            });

            map.insert(file.rel_path.clone(), hashes);
        }
        Ok(map)
    }

    // ── Internal: Copy ───────────────────────────────────────────────

    /// Copy every source file to all destinations. Returns inline hashes.
    /// Supports chunk-level pause/cancel and intra-file progress reporting.
    async fn copy_all_files(
        &self,
        files: &[SourceFile],
        start: &Instant,
        total_bytes: u64,
    ) -> Result<HashMap<String, Vec<HashResult>>> {
        let mut copy_hashes: HashMap<String, Vec<HashResult>> = HashMap::new();
        let mut completed_bytes: u64 = 0;
        let total_files = files.len();

        let copy_config = CopyEngineConfig {
            buffer_size: self.config.buffer_size,
            max_retries: self.config.max_retries,
            cascading_enabled: false,
            hash_algorithms: self.config.hash_algorithms.clone(),
            conflict_policy: copy_engine::FileConflictPolicy::SkipIfSameSize,
        };

        for (i, file) in files.iter().enumerate() {
            // Apply conflict resolutions to destination paths
            let mut dest_files: Vec<PathBuf> = Vec::new();
            let mut has_overwrite = false;

            for d in &self.config.dest_paths {
                let dest = d.join(&file.rel_path);
                let key = format!("{}::{}", file.rel_path, dest.to_string_lossy());
                match self.config.conflict_resolutions.get(&key) {
                    Some(ConflictAction::Skip) => {
                        // Skip this destination for this file
                        self.emit(OffloadEvent::FileSkipped {
                            rel_path: file.rel_path.clone(),
                            reason: format!(
                                "skipped by user ({})",
                                d.file_name().unwrap_or_default().to_string_lossy()
                            ),
                        });
                        continue;
                    }
                    Some(ConflictAction::Overwrite) => {
                        has_overwrite = true;
                        dest_files.push(dest);
                    }
                    Some(ConflictAction::KeepBoth) => {
                        // Rename: file.ext → file_copy.ext (or file_copy_2.ext if that exists)
                        let renamed = generate_keep_both_path(&dest);
                        // Update DB record to match the renamed destination path
                        {
                            let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                            conn.execute(
                                "UPDATE copy_tasks SET dest_path = ?1, updated_at = datetime('now')
                                 WHERE job_id = ?2 AND source_path = ?3 AND dest_path = ?4",
                                rusqlite::params![
                                    renamed.to_string_lossy().as_ref(),
                                    self.config.job_id,
                                    file.abs_path.to_string_lossy().as_ref(),
                                    dest.to_string_lossy().as_ref(),
                                ],
                            )?;
                        }
                        dest_files.push(renamed);
                    }
                    None => {
                        // No conflict resolution — copy normally
                        dest_files.push(dest);
                    }
                }
            }

            if dest_files.is_empty() {
                // All destinations were skipped for this file
                completed_bytes += file.size;
                self.emit(OffloadEvent::FileCopyCompleted {
                    rel_path: file.rel_path.clone(),
                    file_size: file.size,
                    hashes: vec![],
                    file_index: i,
                    total_files,
                });
                self.emit(OffloadEvent::JobProgress {
                    completed_files: i + 1,
                    total_files,
                    completed_bytes,
                    total_bytes,
                    phase: OffloadPhase::Copying,
                    elapsed_secs: start.elapsed().as_secs_f64(),
                    message: None,
                });
                continue;
            }

            // If any destination has Overwrite resolution, use Overwrite policy for the multi-copy.
            // This is safe because: Skip dests are already removed, KeepBoth dests are renamed,
            // and remaining non-conflict dests won't have matching files to overwrite.
            let file_copy_config = if has_overwrite {
                CopyEngineConfig {
                    conflict_policy: copy_engine::FileConflictPolicy::Overwrite,
                    ..copy_config.clone()
                }
            } else {
                copy_config.clone()
            };

            // Build per-file status message (sent via JobProgress, not PhaseChanged)
            // Use last 2 path segments for dest display (e.g. "Volumes/SHUTTLE_01")
            let dest_names: Vec<String> = self
                .config
                .dest_paths
                .iter()
                .map(|d| {
                    let comps: Vec<&str> = d
                        .components()
                        .filter_map(|c| c.as_os_str().to_str())
                        .collect();
                    let n = comps.len();
                    if n >= 2 {
                        format!("{}/{}", comps[n - 2], comps[n - 1])
                    } else {
                        d.to_string_lossy().to_string()
                    }
                })
                .collect();
            let file_msg = format!(
                "Copying ({}/{}) {} → {}",
                i + 1,
                total_files,
                file.rel_path,
                dest_names.join(", ")
            );

            self.emit(OffloadEvent::FileCopyStarted {
                rel_path: file.rel_path.clone(),
                file_size: file.size,
                dest_count: dest_files.len(),
            });

            // Mark tasks as copying in DB
            {
                let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                for dest in &dest_files {
                    conn.execute(
                        "UPDATE copy_tasks SET status = ?1, updated_at = datetime('now')
                         WHERE job_id = ?2 AND source_path = ?3 AND dest_path = ?4",
                        rusqlite::params![
                            STATUS_COPYING,
                            self.config.job_id,
                            file.abs_path.to_string_lossy().as_ref(),
                            dest.to_string_lossy().as_ref(),
                        ],
                    )?;
                }
            } // lock released

            // Set up intra-file progress reporting (throttled to 100ms for responsive speed chart)
            let event_tx = self.event_tx.clone();
            let file_completed_bytes = completed_bytes;
            let progress_total_files = total_files;
            let progress_file_index = i;
            let progress_start = *start;
            let progress_msg = file_msg.clone();
            let last_progress_emit =
                std::sync::Mutex::new(Instant::now() - std::time::Duration::from_secs(1));
            let progress_callback = move |file_bytes_written: u64, _file_total: u64| {
                let mut last = last_progress_emit.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed() >= std::time::Duration::from_millis(100) {
                    let current_total = file_completed_bytes + file_bytes_written;
                    event_tx
                        .send(OffloadEvent::JobProgress {
                            completed_files: progress_file_index,
                            total_files: progress_total_files,
                            completed_bytes: current_total,
                            total_bytes,
                            phase: OffloadPhase::Copying,
                            elapsed_secs: progress_start.elapsed().as_secs_f64(),
                            message: Some(progress_msg.clone()),
                        })
                        .ok();
                    *last = Instant::now();
                }
            };

            let control = copy_engine::CopyControl {
                cancel_flag: Some(self.cancel.as_atomic()),
                pause_flag: Some(self.pause.as_atomic()),
                on_progress: Some(Box::new(progress_callback)),
            };

            // Read source once → write to all destinations
            let copy_ok = match copy_engine::copy_file_multi(
                &file.abs_path,
                &dest_files,
                &file_copy_config,
                &control,
            )
            .await
            {
                Ok(results) => {
                    // Check if all results are skipped
                    let all_skipped = results.iter().all(|r| r.skipped);

                    if all_skipped {
                        // All destinations already had this file — emit skip event
                        self.emit(OffloadEvent::FileSkipped {
                            rel_path: file.rel_path.clone(),
                            reason: "destination file exists with matching size".into(),
                        });
                        // Mark tasks as completed (already existed)
                        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                        for r in &results {
                            let task_id: Option<String> = conn
                                .query_row(
                                    "SELECT id FROM copy_tasks
                                     WHERE job_id = ?1 AND source_path = ?2 AND dest_path = ?3",
                                    rusqlite::params![
                                        self.config.job_id,
                                        file.abs_path.to_string_lossy().as_ref(),
                                        r.dest_path.to_string_lossy().as_ref(),
                                    ],
                                    |row| row.get(0),
                                )
                                .ok();
                            if let Some(tid) = task_id {
                                checkpoint::update_task_completed(&conn, &tid, None, None)?;
                            }
                        }
                    } else {
                        // At least some files were actually copied
                        if let Some(first_copied) = results.iter().find(|r| !r.skipped) {
                            copy_hashes
                                .insert(file.rel_path.clone(), first_copied.hash_results.clone());
                        }

                        // Update each per-destination task in DB
                        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                        for r in &results {
                            if r.skipped {
                                // Mark skipped task as completed
                                let task_id: Option<String> = conn
                                    .query_row(
                                        "SELECT id FROM copy_tasks
                                         WHERE job_id = ?1 AND source_path = ?2 AND dest_path = ?3",
                                        rusqlite::params![
                                            self.config.job_id,
                                            file.abs_path.to_string_lossy().as_ref(),
                                            r.dest_path.to_string_lossy().as_ref(),
                                        ],
                                        |row| row.get(0),
                                    )
                                    .ok();
                                if let Some(tid) = task_id {
                                    checkpoint::update_task_completed(&conn, &tid, None, None)?;
                                }
                                continue;
                            }

                            let xxh64 = r
                                .hash_results
                                .iter()
                                .find(|h| h.algorithm == HashAlgorithm::XXH64)
                                .map(|h| h.hex_digest.as_str());
                            let sha256 = r
                                .hash_results
                                .iter()
                                .find(|h| h.algorithm == HashAlgorithm::SHA256)
                                .map(|h| h.hex_digest.as_str());

                            let task_id: Option<String> = conn
                                .query_row(
                                    "SELECT id FROM copy_tasks
                                     WHERE job_id = ?1 AND source_path = ?2 AND dest_path = ?3",
                                    rusqlite::params![
                                        self.config.job_id,
                                        file.abs_path.to_string_lossy().as_ref(),
                                        r.dest_path.to_string_lossy().as_ref(),
                                    ],
                                    |row| row.get(0),
                                )
                                .ok();

                            if let Some(tid) = task_id {
                                if r.success {
                                    checkpoint::update_task_completed(&conn, &tid, xxh64, sha256)?;
                                } else {
                                    checkpoint::update_task_failed(
                                        &conn,
                                        &tid,
                                        r.error.as_deref().unwrap_or("Unknown copy error"),
                                    )?;
                                }
                            }
                        }
                    }
                    true
                }
                Err(e) => {
                    // If cancelled, propagate the error immediately
                    let err_str = e.to_string();
                    if err_str.contains("cancelled by user") {
                        return Err(e);
                    }

                    self.emit(OffloadEvent::Warning {
                        message: format!("Copy failed for {}: {}", file.rel_path, e),
                    });
                    // Mark all tasks for this file as failed
                    let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                    for dest in &dest_files {
                        let task_id: Option<String> = conn
                            .query_row(
                                "SELECT id FROM copy_tasks
                                 WHERE job_id = ?1 AND source_path = ?2 AND dest_path = ?3",
                                rusqlite::params![
                                    self.config.job_id,
                                    file.abs_path.to_string_lossy().as_ref(),
                                    dest.to_string_lossy().as_ref(),
                                ],
                                |row| row.get(0),
                            )
                            .ok();
                        if let Some(tid) = task_id {
                            checkpoint::update_task_failed(&conn, &tid, &e.to_string())?;
                        }
                    }
                    false
                }
            };

            // Only count completed bytes for successful copies (avoid inflated progress)
            if copy_ok {
                completed_bytes += file.size;
            }

            self.emit(OffloadEvent::FileCopyCompleted {
                rel_path: file.rel_path.clone(),
                file_size: file.size,
                hashes: copy_hashes.get(&file.rel_path).cloned().unwrap_or_default(),
                file_index: i,
                total_files,
            });

            self.emit(OffloadEvent::JobProgress {
                completed_files: i + 1,
                total_files,
                completed_bytes,
                total_bytes,
                phase: OffloadPhase::Copying,
                elapsed_secs: start.elapsed().as_secs_f64(),
                message: Some(file_msg),
            });

            // Check for pause/cancel after each file (chunk-level checks also in copy_engine)
            self.check_paused().await?;
        }

        Ok(copy_hashes)
    }

    // ── Internal: Cascade Copy ─────────────────────────────────────────

    /// Phase 3a (cascade mode): Copy source files to the PRIMARY destination only.
    /// Returns inline hashes from the copy.
    async fn copy_to_primary(
        &self,
        files: &[SourceFile],
        start: &Instant,
        total_bytes: u64,
    ) -> Result<HashMap<String, Vec<HashResult>>> {
        let primary_dest = &self.config.dest_paths[0];
        let mut copy_hashes: HashMap<String, Vec<HashResult>> = HashMap::new();
        let mut completed_bytes: u64 = 0;
        let total_files = files.len();

        let copy_config = CopyEngineConfig {
            buffer_size: self.config.buffer_size,
            max_retries: self.config.max_retries,
            cascading_enabled: true,
            hash_algorithms: self.config.hash_algorithms.clone(),
            conflict_policy: copy_engine::FileConflictPolicy::SkipIfSameSize,
        };

        for (i, file) in files.iter().enumerate() {
            let dest_file = primary_dest.join(&file.rel_path);

            self.emit(OffloadEvent::FileCopyStarted {
                rel_path: file.rel_path.clone(),
                file_size: file.size,
                dest_count: 1,
            });

            // Mark task as copying
            {
                let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                conn.execute(
                    "UPDATE copy_tasks SET status = ?1, updated_at = datetime('now')
                     WHERE job_id = ?2 AND source_path = ?3 AND dest_path = ?4",
                    rusqlite::params![
                        STATUS_COPYING,
                        self.config.job_id,
                        file.abs_path.to_string_lossy().as_ref(),
                        dest_file.to_string_lossy().as_ref(),
                    ],
                )?;
            }

            let control = copy_engine::CopyControl {
                cancel_flag: Some(self.cancel.as_atomic()),
                pause_flag: Some(self.pause.as_atomic()),
                on_progress: None, // cascade doesn't need intra-file progress
            };

            match copy_engine::copy_file_single(&file.abs_path, &dest_file, &copy_config, &control)
                .await
            {
                Ok(result) => {
                    copy_hashes.insert(file.rel_path.clone(), result.hash_results.clone());

                    let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                    let xxh64 = result
                        .hash_results
                        .iter()
                        .find(|h| h.algorithm == HashAlgorithm::XXH64)
                        .map(|h| h.hex_digest.as_str());
                    let sha256 = result
                        .hash_results
                        .iter()
                        .find(|h| h.algorithm == HashAlgorithm::SHA256)
                        .map(|h| h.hex_digest.as_str());

                    let task_id: Option<String> = conn
                        .query_row(
                            "SELECT id FROM copy_tasks
                             WHERE job_id = ?1 AND source_path = ?2 AND dest_path = ?3",
                            rusqlite::params![
                                self.config.job_id,
                                file.abs_path.to_string_lossy().as_ref(),
                                dest_file.to_string_lossy().as_ref(),
                            ],
                            |row| row.get(0),
                        )
                        .ok();

                    if let Some(tid) = task_id {
                        checkpoint::update_task_completed(&conn, &tid, xxh64, sha256)?;
                    }
                }
                Err(e) => {
                    // Check if cancelled — propagate immediately
                    let err_str = e.to_string();
                    if err_str.contains("cancelled by user") {
                        return Err(e);
                    }
                    self.emit(OffloadEvent::Warning {
                        message: format!("Primary copy failed for {}: {}", file.rel_path, e),
                    });
                    let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                    let task_id: Option<String> = conn
                        .query_row(
                            "SELECT id FROM copy_tasks
                             WHERE job_id = ?1 AND source_path = ?2 AND dest_path = ?3",
                            rusqlite::params![
                                self.config.job_id,
                                file.abs_path.to_string_lossy().as_ref(),
                                dest_file.to_string_lossy().as_ref(),
                            ],
                            |row| row.get(0),
                        )
                        .ok();
                    if let Some(tid) = task_id {
                        checkpoint::update_task_failed(&conn, &tid, &e.to_string())?;
                    }
                }
            }

            completed_bytes += file.size;

            self.emit(OffloadEvent::FileCopyCompleted {
                rel_path: file.rel_path.clone(),
                file_size: file.size,
                hashes: copy_hashes.get(&file.rel_path).cloned().unwrap_or_default(),
                file_index: i,
                total_files,
            });

            self.emit(OffloadEvent::JobProgress {
                completed_files: i + 1,
                total_files,
                completed_bytes,
                total_bytes,
                phase: OffloadPhase::Copying,
                elapsed_secs: start.elapsed().as_secs_f64(),
                message: None,
            });

            // Check for pause/cancel after each file
            self.check_paused().await?;
        }

        Ok(copy_hashes)
    }

    /// Phase 3b (cascade mode): Copy from PRIMARY destination → all SECONDARY destinations.
    /// The source card is now free to be ejected.
    async fn cascade_from_primary(
        &self,
        files: &[SourceFile],
        start: &Instant,
        total_bytes: u64,
    ) -> Result<()> {
        let primary_dest = &self.config.dest_paths[0];
        let secondary_dests = &self.config.dest_paths[1..];
        let total_files = files.len();
        let mut completed_bytes: u64 = 0;

        let copy_config = CopyEngineConfig {
            buffer_size: self.config.buffer_size,
            max_retries: self.config.max_retries,
            cascading_enabled: true,
            hash_algorithms: self.config.hash_algorithms.clone(),
            conflict_policy: copy_engine::FileConflictPolicy::SkipIfSameSize,
        };

        for (i, file) in files.iter().enumerate() {
            let primary_file = primary_dest.join(&file.rel_path);
            let secondary_files: Vec<PathBuf> = secondary_dests
                .iter()
                .map(|d| d.join(&file.rel_path))
                .collect();

            self.emit(OffloadEvent::FileCopyStarted {
                rel_path: file.rel_path.clone(),
                file_size: file.size,
                dest_count: secondary_files.len(),
            });

            // Mark secondary tasks as copying
            {
                let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                for dest in &secondary_files {
                    conn.execute(
                        "UPDATE copy_tasks SET status = ?1, updated_at = datetime('now')
                         WHERE job_id = ?2 AND source_path = ?3 AND dest_path = ?4",
                        rusqlite::params![
                            STATUS_COPYING,
                            self.config.job_id,
                            file.abs_path.to_string_lossy().as_ref(),
                            dest.to_string_lossy().as_ref(),
                        ],
                    )?;
                }
            }

            let control = copy_engine::CopyControl {
                cancel_flag: Some(self.cancel.as_atomic()),
                pause_flag: Some(self.pause.as_atomic()),
                on_progress: None, // cascade doesn't need intra-file progress
            };

            // Read from primary copy → write to all secondary destinations
            match copy_engine::copy_file_multi(
                &primary_file,
                &secondary_files,
                &copy_config,
                &control,
            )
            .await
            {
                Ok(results) => {
                    let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                    for r in &results {
                        let xxh64 = r
                            .hash_results
                            .iter()
                            .find(|h| h.algorithm == HashAlgorithm::XXH64)
                            .map(|h| h.hex_digest.as_str());
                        let sha256 = r
                            .hash_results
                            .iter()
                            .find(|h| h.algorithm == HashAlgorithm::SHA256)
                            .map(|h| h.hex_digest.as_str());

                        let task_id: Option<String> = conn
                            .query_row(
                                "SELECT id FROM copy_tasks
                                 WHERE job_id = ?1 AND source_path = ?2 AND dest_path = ?3",
                                rusqlite::params![
                                    self.config.job_id,
                                    file.abs_path.to_string_lossy().as_ref(),
                                    r.dest_path.to_string_lossy().as_ref(),
                                ],
                                |row| row.get(0),
                            )
                            .ok();

                        if let Some(tid) = task_id {
                            if r.success {
                                checkpoint::update_task_completed(&conn, &tid, xxh64, sha256)?;
                            } else {
                                checkpoint::update_task_failed(
                                    &conn,
                                    &tid,
                                    r.error.as_deref().unwrap_or("Cascade copy error"),
                                )?;
                            }
                        }
                    }
                }
                Err(e) => {
                    // Check if cancelled — propagate immediately
                    let err_str = e.to_string();
                    if err_str.contains("cancelled by user") {
                        return Err(e);
                    }
                    self.emit(OffloadEvent::Warning {
                        message: format!("Cascade failed for {}: {}", file.rel_path, e),
                    });
                    let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                    for dest in &secondary_files {
                        let task_id: Option<String> = conn
                            .query_row(
                                "SELECT id FROM copy_tasks
                                 WHERE job_id = ?1 AND source_path = ?2 AND dest_path = ?3",
                                rusqlite::params![
                                    self.config.job_id,
                                    file.abs_path.to_string_lossy().as_ref(),
                                    dest.to_string_lossy().as_ref(),
                                ],
                                |row| row.get(0),
                            )
                            .ok();
                        if let Some(tid) = task_id {
                            checkpoint::update_task_failed(&conn, &tid, &e.to_string())?;
                        }
                    }
                }
            }

            completed_bytes += file.size;

            self.emit(OffloadEvent::FileCopyCompleted {
                rel_path: file.rel_path.clone(),
                file_size: file.size,
                hashes: Vec::new(), // Cascade hashes already validated
                file_index: i,
                total_files,
            });

            self.emit(OffloadEvent::JobProgress {
                completed_files: i + 1,
                total_files,
                completed_bytes,
                total_bytes,
                phase: OffloadPhase::Cascading,
                elapsed_secs: start.elapsed().as_secs_f64(),
                message: None,
            });

            // Check for pause/cancel after each file
            self.check_paused().await?;
        }

        Ok(())
    }

    // ── Internal: Verify ─────────────────────────────────────────────

    /// Re-read destination files and compare hashes against source.
    async fn verify_destinations(
        &self,
        files: &[SourceFile],
        source_hashes: &HashMap<String, Vec<HashResult>>,
        errors: &mut Vec<String>,
    ) -> Result<usize> {
        let cfg = HashEngineConfig {
            algorithms: self.config.hash_algorithms.clone(),
            buffer_size: self.config.buffer_size,
        };
        let mut failed = 0;
        let total_files = files.len();
        let total_bytes: u64 = files.iter().map(|f| f.size).sum();
        let total_verify_ops = total_files * self.config.dest_paths.len();
        let mut completed_ops: usize = 0;
        let mut completed_bytes: u64 = 0;
        let verify_start = Instant::now();

        for (fi, file) in files.iter().enumerate() {
            // Check pause/cancel between files
            self.check_paused().await?;

            let expected = match source_hashes.get(&file.rel_path) {
                Some(h) => h,
                None => {
                    self.emit(OffloadEvent::Warning {
                        message: format!("No source hash for {}, skipping verify", file.rel_path),
                    });
                    continue;
                }
            };

            for dest_root in &self.config.dest_paths {
                self.check_cancelled()?;
                let dest_file = dest_root.join(&file.rel_path);

                let verify_msg = format!(
                    "Verifying ({}/{}) {}...",
                    completed_ops + 1,
                    total_verify_ops,
                    file.rel_path
                );

                if !dest_file.exists() {
                    failed += 1;
                    errors.push(format!("Missing: {:?}", dest_file));
                    self.emit(OffloadEvent::FileVerified {
                        rel_path: file.rel_path.clone(),
                        dest_path: dest_file.to_string_lossy().into(),
                        verified: false,
                        mismatch_detail: Some("File not found after copy".into()),
                    });
                    completed_ops += 1;
                    continue;
                }

                // Progress callback for intra-file verify progress
                let event_tx = self.event_tx.clone();
                let base_completed_bytes = completed_bytes;
                let verify_start_clone = verify_start;
                let total_files_for_progress = total_files;
                let fi_for_progress = fi;
                let progress_msg = verify_msg.clone();
                let last_emit =
                    std::sync::Mutex::new(Instant::now() - std::time::Duration::from_secs(1));
                let verify_progress = move |bytes_hashed: u64, _file_total: u64| {
                    let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
                    if last.elapsed() >= std::time::Duration::from_millis(100) {
                        event_tx
                            .send(OffloadEvent::JobProgress {
                                completed_files: fi_for_progress,
                                total_files: total_files_for_progress,
                                completed_bytes: base_completed_bytes + bytes_hashed,
                                total_bytes,
                                phase: OffloadPhase::Verifying,
                                elapsed_secs: verify_start_clone.elapsed().as_secs_f64(),
                                message: Some(progress_msg.clone()),
                            })
                            .ok();
                        *last = Instant::now();
                    }
                };

                let actual =
                    hash_engine::hash_file_with_progress(&dest_file, &cfg, Some(&verify_progress))
                        .await
                        .with_context(|| format!("Verify read failed: {:?}", dest_file))?;

                completed_ops += 1;

                let mut ok = true;
                let mut detail = None;

                for exp in expected {
                    if let Some(act) = actual.iter().find(|h| h.algorithm == exp.algorithm) {
                        if act.hex_digest != exp.hex_digest {
                            ok = false;
                            detail = Some(format!(
                                "{}: expected {} got {}",
                                exp.algorithm, exp.hex_digest, act.hex_digest
                            ));
                            break;
                        }
                    }
                }

                if !ok {
                    failed += 1;
                    errors.push(format!(
                        "Hash mismatch: {} → {:?}: {}",
                        file.rel_path,
                        dest_file,
                        detail.as_deref().unwrap_or("unknown")
                    ));
                }

                self.emit(OffloadEvent::FileVerified {
                    rel_path: file.rel_path.clone(),
                    dest_path: dest_file.to_string_lossy().into(),
                    verified: ok,
                    mismatch_detail: detail,
                });
            }

            // Update completed bytes for this source file (count once even with multiple dests)
            completed_bytes += file.size;

            // Emit progress after each source file is fully verified
            self.emit(OffloadEvent::JobProgress {
                completed_files: fi + 1,
                total_files,
                completed_bytes,
                total_bytes,
                phase: OffloadPhase::Verifying,
                elapsed_secs: verify_start.elapsed().as_secs_f64(),
                message: None,
            });
        }

        Ok(failed)
    }

    // ── Internal: MHL Seal ───────────────────────────────────────────

    /// Generate ASC MHL manifests for each destination.
    async fn seal_mhl(
        &self,
        files: &[SourceFile],
        source_hashes: &HashMap<String, Vec<HashResult>>,
    ) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();

        for dest_root in &self.config.dest_paths {
            let mut file_hashes: HashMap<String, Vec<HashResult>> = HashMap::new();
            let mut file_metadata: HashMap<String, (u64, chrono::DateTime<chrono::Utc>)> =
                HashMap::new();

            for file in files {
                if let Some(h) = source_hashes.get(&file.rel_path) {
                    file_hashes.insert(file.rel_path.clone(), h.clone());
                }

                let dest_file = dest_root.join(&file.rel_path);
                let modified = tokio::fs::metadata(&dest_file)
                    .await
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(chrono::DateTime::<chrono::Utc>::from)
                    .unwrap_or_else(chrono::Utc::now);

                file_metadata.insert(file.rel_path.clone(), (file.size, modified));
            }

            let mut history = mhl::load_or_create_history(dest_root).await?;
            let mhl_config = MhlConfig::default();

            match mhl::create_generation(
                &mut history,
                &file_hashes,
                &file_metadata,
                MhlProcessType::Transfer,
                &mhl_config,
            )
            .await
            {
                Ok(path) => paths.push(path),
                Err(e) => {
                    self.emit(OffloadEvent::Warning {
                        message: format!("MHL failed for {:?}: {}", dest_root, e),
                    });
                }
            }
        }

        Ok(paths)
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

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

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;

    /// Create an in-memory DB with the same schema as production.
    fn test_db() -> Arc<std::sync::Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY, name TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                source_path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS copy_tasks (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL REFERENCES jobs(id),
                source_path TEXT NOT NULL, dest_path TEXT NOT NULL,
                file_size INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'pending',
                hash_xxh64 TEXT, hash_sha256 TEXT,
                error_msg TEXT, retry_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .unwrap();
        Arc::new(std::sync::Mutex::new(conn))
    }

    /// Helper: write test files in a temp source directory
    fn create_source_files(dir: &Path, files: &[(&str, &[u8])]) {
        for (name, content) in files {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(content).unwrap();
        }
    }

    /// Collect all events from the receiver until it closes
    async fn drain_events(mut rx: mpsc::UnboundedReceiver<OffloadEvent>) -> Vec<OffloadEvent> {
        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }
        events
    }

    #[tokio::test]
    async fn test_full_offload_pipeline() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        let dest1 = tmp.path().join("dest1");
        let dest2 = tmp.path().join("dest2");
        std::fs::create_dir_all(&source).unwrap();

        create_source_files(
            &source,
            &[
                ("clip001.mov", b"video data clip 001"),
                ("clip002.mov", b"video data clip 002 longer"),
                ("audio/boom.wav", b"audio waveform data"),
            ],
        );

        let db = test_db();
        let (tx, rx) = mpsc::unbounded_channel();

        let config = OffloadConfig {
            job_id: "test-job-1".into(),
            job_name: "Day 1 Offload".into(),
            source_path: source.clone(),
            dest_paths: vec![dest1.clone(), dest2.clone()],
            hash_algorithms: vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256],
            buffer_size: 1024,
            source_verify: true,
            post_verify: true,
            generate_mhl: true,
            max_retries: 3,
            cascade: false,
            conflict_resolutions: HashMap::new(),
        };

        let workflow = OffloadWorkflow::new(config, db.clone(), tx);
        let result = workflow.execute().await.unwrap();

        // Verify result
        assert!(result.success);
        assert_eq!(result.total_files, 3);
        assert_eq!(result.failed_files, 0);
        assert_eq!(result.source_hashes.len(), 3);
        assert!(!result.mhl_paths.is_empty());

        // Verify all files were copied correctly
        for name in &["clip001.mov", "clip002.mov", "audio/boom.wav"] {
            let src_data = std::fs::read(source.join(name)).unwrap();
            let d1_data = std::fs::read(dest1.join(name)).unwrap();
            let d2_data = std::fs::read(dest2.join(name)).unwrap();
            assert_eq!(src_data, d1_data, "dest1 mismatch for {}", name);
            assert_eq!(src_data, d2_data, "dest2 mismatch for {}", name);
        }

        // Verify MHL was generated
        assert!(dest1.join("ascmhl").exists());
        assert!(dest2.join("ascmhl").exists());

        // Verify DB records
        {
            let conn = db.lock().unwrap();
            let status: String = conn
                .query_row("SELECT status FROM jobs WHERE id = 'test-job-1'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(status, "completed");

            let completed: usize = conn
                .query_row(
                    "SELECT COUNT(*) FROM copy_tasks WHERE job_id = 'test-job-1' AND status = 'completed'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            // 3 files × 2 destinations = 6 tasks
            assert_eq!(completed, 6);
        }

        // Verify we got meaningful events
        drop(workflow);
        let events = drain_events(rx).await;
        let phase_changes: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, OffloadEvent::PhaseChanged { .. }))
            .collect();
        assert!(phase_changes.len() >= 5); // PreFlight, SourceVerify, Copy, Verify, Seal/Complete
    }

    #[tokio::test]
    async fn test_offload_without_source_verify() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("src");
        let dest = tmp.path().join("dst");
        std::fs::create_dir_all(&source).unwrap();

        create_source_files(&source, &[("file.bin", b"binary content 12345")]);

        let db = test_db();
        let (tx, _rx) = mpsc::unbounded_channel();

        let config = OffloadConfig {
            job_id: "test-no-srcverify".into(),
            job_name: "Quick Copy".into(),
            source_path: source,
            dest_paths: vec![dest.clone()],
            hash_algorithms: vec![HashAlgorithm::XXH64],
            buffer_size: 1024,
            source_verify: false,
            post_verify: true,
            generate_mhl: false,
            max_retries: 1,
            cascade: false,
            conflict_resolutions: HashMap::new(),
        };

        let workflow = OffloadWorkflow::new(config, db, tx);
        let result = workflow.execute().await.unwrap();

        assert!(result.success);
        assert_eq!(result.total_files, 1);
        // Even without explicit source verify, inline copy hashes are used
        assert_eq!(result.source_hashes.len(), 1);
        assert_eq!(
            std::fs::read(dest.join("file.bin")).unwrap(),
            b"binary content 12345"
        );
    }

    #[tokio::test]
    async fn test_offload_post_verify_detects_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("src");
        let dest = tmp.path().join("dst");
        std::fs::create_dir_all(&source).unwrap();

        create_source_files(&source, &[("data.raw", b"original raw camera data")]);

        let db = test_db();
        let (tx, _rx) = mpsc::unbounded_channel();

        let config = OffloadConfig {
            job_id: "test-corrupt".into(),
            job_name: "Corrupt Test".into(),
            source_path: source,
            dest_paths: vec![dest.clone()],
            hash_algorithms: vec![HashAlgorithm::SHA256],
            buffer_size: 1024,
            source_verify: true,
            post_verify: true,
            generate_mhl: false,
            max_retries: 1,
            cascade: false,
            conflict_resolutions: HashMap::new(),
        };

        let workflow = OffloadWorkflow::new(config, db, tx);
        let result = workflow.execute().await.unwrap();

        // Copy should succeed initially
        assert!(result.success);

        // Now corrupt the destination file and run verify again manually
        std::fs::write(dest.join("data.raw"), b"CORRUPTED!!!").unwrap();

        // Re-read and verify
        let cfg = HashEngineConfig {
            algorithms: vec![HashAlgorithm::SHA256],
            buffer_size: 1024,
        };
        let original_hash = &result.source_hashes["data.raw"];
        let corrupted_hash = hash_engine::hash_file(&dest.join("data.raw"), &cfg)
            .await
            .unwrap();

        assert_ne!(
            original_hash[0].hex_digest, corrupted_hash[0].hex_digest,
            "Corruption should produce different hash"
        );
    }

    #[tokio::test]
    async fn test_offload_ignores_ds_store() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("src");
        let dest = tmp.path().join("dst");
        std::fs::create_dir_all(&source).unwrap();

        create_source_files(
            &source,
            &[
                ("clip.mov", b"video"),
                (".DS_Store", b"mac junk"),
                ("Thumbs.db", b"windows junk"),
            ],
        );

        let db = test_db();
        let (tx, _rx) = mpsc::unbounded_channel();

        let config = OffloadConfig {
            job_id: "test-ignore".into(),
            job_name: "Ignore Test".into(),
            source_path: source,
            dest_paths: vec![dest.clone()],
            hash_algorithms: vec![HashAlgorithm::XXH64],
            buffer_size: 1024,
            source_verify: false,
            post_verify: false,
            generate_mhl: false,
            max_retries: 1,
            cascade: false,
            conflict_resolutions: HashMap::new(),
        };

        let workflow = OffloadWorkflow::new(config, db, tx);
        let result = workflow.execute().await.unwrap();

        assert!(result.success);
        // Only clip.mov should be copied (DS_Store and Thumbs.db ignored)
        assert_eq!(result.total_files, 1);
        assert!(dest.join("clip.mov").exists());
        assert!(!dest.join(".DS_Store").exists());
        assert!(!dest.join("Thumbs.db").exists());
    }

    #[tokio::test]
    async fn test_offload_empty_source_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("empty_src");
        let dest = tmp.path().join("dst");
        std::fs::create_dir_all(&source).unwrap();

        let db = test_db();
        let (tx, _rx) = mpsc::unbounded_channel();

        let config = OffloadConfig {
            job_id: "test-empty".into(),
            job_name: "Empty".into(),
            source_path: source,
            dest_paths: vec![dest],
            source_verify: false,
            post_verify: false,
            generate_mhl: false,
            ..Default::default()
        };

        let workflow = OffloadWorkflow::new(config, db, tx);
        let err = workflow.execute().await.unwrap_err();
        assert!(err.to_string().contains("No files found"));
    }

    #[tokio::test]
    async fn test_offload_multi_dest_consistency() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("src");
        let d1 = tmp.path().join("d1");
        let d2 = tmp.path().join("d2");
        let d3 = tmp.path().join("d3");
        std::fs::create_dir_all(&source).unwrap();

        create_source_files(
            &source,
            &[
                ("A001C001.mov", b"ARRI footage A001C001"),
                ("A001C002.mov", b"ARRI footage A001C002 take 2"),
            ],
        );

        let db = test_db();
        let (tx, _rx) = mpsc::unbounded_channel();

        let config = OffloadConfig {
            job_id: "test-3dest".into(),
            job_name: "Triple Backup".into(),
            source_path: source.clone(),
            dest_paths: vec![d1.clone(), d2.clone(), d3.clone()],
            hash_algorithms: vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256],
            buffer_size: 1024,
            source_verify: true,
            post_verify: true,
            generate_mhl: false,
            max_retries: 1,
            cascade: false,
            conflict_resolutions: HashMap::new(),
        };

        let workflow = OffloadWorkflow::new(config, db.clone(), tx);
        let result = workflow.execute().await.unwrap();

        assert!(result.success);
        assert_eq!(result.total_files, 2);

        // All three destinations must match source exactly
        for name in &["A001C001.mov", "A001C002.mov"] {
            let src = std::fs::read(source.join(name)).unwrap();
            assert_eq!(src, std::fs::read(d1.join(name)).unwrap());
            assert_eq!(src, std::fs::read(d2.join(name)).unwrap());
            assert_eq!(src, std::fs::read(d3.join(name)).unwrap());
        }

        // 2 files × 3 dests = 6 tasks all completed
        let conn = db.lock().unwrap();
        let count: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM copy_tasks WHERE job_id = 'test-3dest' AND status = 'completed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 6);
    }

    #[tokio::test]
    async fn test_cascading_copy_primary_then_secondary() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("src");
        let primary = tmp.path().join("fast_ssd");
        let secondary1 = tmp.path().join("shuttle1");
        let secondary2 = tmp.path().join("shuttle2");
        std::fs::create_dir_all(&source).unwrap();

        create_source_files(
            &source,
            &[
                ("A001C001.braw", b"BMPCC footage clip 001 take 1"),
                ("A001C002.braw", b"BMPCC footage clip 002 take 1 longer"),
                ("audio/boom.wav", b"audio boom track day 1"),
            ],
        );

        let db = test_db();
        let (tx, rx) = mpsc::unbounded_channel();

        let config = OffloadConfig {
            job_id: "test-cascade".into(),
            job_name: "Cascade Day 1".into(),
            source_path: source.clone(),
            dest_paths: vec![primary.clone(), secondary1.clone(), secondary2.clone()],
            hash_algorithms: vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256],
            buffer_size: 1024,
            source_verify: true,
            post_verify: true,
            generate_mhl: false,
            max_retries: 1,
            cascade: true,
            conflict_resolutions: HashMap::new(),
        };

        let workflow = OffloadWorkflow::new(config, db.clone(), tx);
        let result = workflow.execute().await.unwrap();

        assert!(result.success, "Cascade offload should succeed");
        assert_eq!(result.total_files, 3);
        assert_eq!(result.failed_files, 0);

        // ALL destinations must match source exactly
        for name in &["A001C001.braw", "A001C002.braw", "audio/boom.wav"] {
            let src_data = std::fs::read(source.join(name)).unwrap();
            let pri_data = std::fs::read(primary.join(name)).unwrap();
            let s1_data = std::fs::read(secondary1.join(name)).unwrap();
            let s2_data = std::fs::read(secondary2.join(name)).unwrap();
            assert_eq!(src_data, pri_data, "Primary mismatch for {}", name);
            assert_eq!(src_data, s1_data, "Secondary1 mismatch for {}", name);
            assert_eq!(src_data, s2_data, "Secondary2 mismatch for {}", name);
        }

        // 3 files × 3 dests = 9 tasks all completed
        {
            let conn = db.lock().unwrap();
            let count: usize = conn
                .query_row(
                    "SELECT COUNT(*) FROM copy_tasks WHERE job_id = 'test-cascade' AND status = 'completed'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 9);
        }

        // Verify we got Cascading phase events
        drop(workflow);
        let events = drain_events(rx).await;
        let cascading_events: Vec<_> = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    OffloadEvent::PhaseChanged {
                        phase: OffloadPhase::Cascading,
                        ..
                    }
                )
            })
            .collect();
        assert!(
            !cascading_events.is_empty(),
            "Should have emitted a Cascading phase event"
        );
    }

    #[tokio::test]
    async fn test_cascade_single_dest_falls_back_to_normal() {
        // With only 1 dest, cascade should behave like normal mode
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("src");
        let dest = tmp.path().join("dst");
        std::fs::create_dir_all(&source).unwrap();

        create_source_files(&source, &[("clip.mov", b"single dest cascade test")]);

        let db = test_db();
        let (tx, rx) = mpsc::unbounded_channel();

        let config = OffloadConfig {
            job_id: "test-cascade-single".into(),
            job_name: "Single Dest Cascade".into(),
            source_path: source.clone(),
            dest_paths: vec![dest.clone()],
            hash_algorithms: vec![HashAlgorithm::XXH64],
            buffer_size: 1024,
            source_verify: false,
            post_verify: false,
            generate_mhl: false,
            max_retries: 1,
            cascade: true, // enabled but only 1 dest → should not cascade
            conflict_resolutions: HashMap::new(),
        };

        let workflow = OffloadWorkflow::new(config, db, tx);
        let result = workflow.execute().await.unwrap();

        assert!(result.success);
        assert_eq!(result.total_files, 1);
        assert_eq!(
            std::fs::read(dest.join("clip.mov")).unwrap(),
            b"single dest cascade test"
        );

        // Should NOT have Cascading phase (only 1 dest)
        drop(workflow);
        let events = drain_events(rx).await;
        let cascading_events: Vec<_> = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    OffloadEvent::PhaseChanged {
                        phase: OffloadPhase::Cascading,
                        ..
                    }
                )
            })
            .collect();
        assert!(
            cascading_events.is_empty(),
            "Should NOT cascade when there's only 1 destination"
        );
    }
}
