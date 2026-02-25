//! Tauri Commands — Bridge between Rust backend and React frontend.
//!
//! All functions here are `#[tauri::command]` and can be invoked from
//! TypeScript via `invoke("command_name", { args })`.

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};

use crate::checkpoint::{self, JobProgress};
use crate::config::{self, AppSettings};
use crate::hash_engine::{self, HashAlgorithm, HashEngineConfig, HashResult};
use crate::io_scheduler::IoScheduler;
use crate::mhl::{self, MhlConfig, MhlProcessType};
use crate::volume::{self, DeviceType, VolumeSpaceInfo};
use crate::workflow;

// ─── App State ────────────────────────────────────────────────────────────

/// Shared application state managed by Tauri
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub io_scheduler: Mutex<IoScheduler>,
    pub app_data_dir: PathBuf,
    pub settings: Mutex<AppSettings>,
}

// ─── Response Types ───────────────────────────────────────────────────────

/// Standard response for Tauri commands
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandResult<T: Serialize> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T: Serialize> CommandResult<T> {
    pub fn ok(data: T) -> Self {
        Self { success: true, data: Some(data), error: None }
    }
    pub fn err(msg: String) -> Self {
        Self { success: false, data: None, error: Some(msg) }
    }
}

/// Job info returned to frontend
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub source_path: String,
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub total_bytes: u64,
    pub copied_bytes: u64,
    pub progress_percent: f64,
}

/// Volume info returned to frontend (camelCase for JS)
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VolumeInfoResponse {
    pub id: String,
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub device_type: String,
    pub serial_number: Option<String>,
    pub is_mounted: bool,
    pub usage_percent: f64,
    pub is_low: bool,
    pub is_critical: bool,
}

/// Copy task request from frontend
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateJobRequest {
    pub name: String,
    pub source_path: String,
    pub dest_paths: Vec<String>,
    pub hash_algorithms: Vec<String>,
}

// ─── Job Commands ─────────────────────────────────────────────────────────

/// Create a new offload job. Scans source directory and creates copy tasks.
#[tauri::command]
pub async fn create_job(
    state: State<'_, AppState>,
    request: CreateJobRequest,
) -> Result<CommandResult<JobInfo>, String> {
    let job_id = uuid::Uuid::new_v4().to_string();
    let source = PathBuf::from(&request.source_path);

    if !source.exists() {
        return Ok(CommandResult::err(format!(
            "Source path does not exist: {}",
            request.source_path
        )));
    }

    // Scan source directory for files
    let files = scan_directory(&source).await.map_err(|e| e.to_string())?;
    if files.is_empty() {
        return Ok(CommandResult::err("No files found in source directory".to_string()));
    }

    let _total_bytes: u64 = files.iter().map(|(_, size)| size).sum();

    let conn = state.db.lock().map_err(|e| e.to_string())?;

    // Create the job
    checkpoint::create_job(&conn, &job_id, &request.name, &request.source_path)
        .map_err(|e| e.to_string())?;

    // Create copy tasks for each file × each destination
    for (rel_path, file_size) in &files {
        let source_file = source.join(rel_path);
        for dest_root in &request.dest_paths {
            let dest_file = Path::new(dest_root).join(rel_path);
            let task_id = uuid::Uuid::new_v4().to_string();
            checkpoint::insert_task(
                &conn,
                &task_id,
                &job_id,
                source_file.to_str().unwrap_or(""),
                dest_file.to_str().unwrap_or(""),
                *file_size,
            )
            .map_err(|e| e.to_string())?;
        }
    }

    let progress = checkpoint::get_job_progress(&conn, &job_id).map_err(|e| e.to_string())?;

    Ok(CommandResult::ok(JobInfo {
        id: job_id,
        name: request.name,
        status: "pending".to_string(),
        source_path: request.source_path,
        total_tasks: progress.total_tasks,
        completed_tasks: progress.completed,
        failed_tasks: progress.failed,
        total_bytes: progress.total_bytes,
        copied_bytes: progress.completed_bytes,
        progress_percent: 0.0,
    }))
}

/// Get progress for a specific job
#[tauri::command]
pub fn get_job_progress(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<CommandResult<JobInfo>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let progress = checkpoint::get_job_progress(&conn, &job_id).map_err(|e| e.to_string())?;

    let percent = if progress.total_bytes > 0 {
        (progress.completed_bytes as f64 / progress.total_bytes as f64) * 100.0
    } else {
        0.0
    };

    // Get job info from database
    let (name, source_path, status): (String, String, String) = conn
        .query_row(
            "SELECT name, source_path, status FROM jobs WHERE id = ?1",
            rusqlite::params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;

    Ok(CommandResult::ok(JobInfo {
        id: job_id,
        name,
        status,
        source_path,
        total_tasks: progress.total_tasks,
        completed_tasks: progress.completed,
        failed_tasks: progress.failed,
        total_bytes: progress.total_bytes,
        copied_bytes: progress.completed_bytes,
        progress_percent: percent,
    }))
}

/// List all jobs
#[tauri::command]
pub fn list_jobs(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<JobInfo>>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT id, name, source_path, status FROM jobs ORDER BY created_at DESC")
        .map_err(|e| e.to_string())?;

    let jobs: Vec<(String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut result = Vec::new();
    for (id, name, source_path, status) in jobs {
        let progress = checkpoint::get_job_progress(&conn, &id).unwrap_or(JobProgress {
            job_id: id.clone(),
            total_tasks: 0,
            completed: 0,
            pending: 0,
            copying: 0,
            failed: 0,
            total_bytes: 0,
            completed_bytes: 0,
        });

        let percent = if progress.total_bytes > 0 {
            (progress.completed_bytes as f64 / progress.total_bytes as f64) * 100.0
        } else {
            0.0
        };

        result.push(JobInfo {
            id,
            name,
            status,
            source_path,
            total_tasks: progress.total_tasks,
            completed_tasks: progress.completed,
            failed_tasks: progress.failed,
            total_bytes: progress.total_bytes,
            copied_bytes: progress.completed_bytes,
            progress_percent: percent,
        });
    }

    Ok(CommandResult::ok(result))
}

/// Recover a job after interruption (clean .tmp files, reset tasks)
#[tauri::command]
pub async fn recover_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<CommandResult<JobInfo>, String> {
    // Step 1: Get interrupted tasks and reset them (sync, release lock before await)
    let interrupted_dest_paths: Vec<String> = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let interrupted = checkpoint::get_interrupted_tasks(&conn, &job_id)
            .map_err(|e| e.to_string())?;

        let paths: Vec<String> = interrupted.iter().map(|t| t.dest_path.clone()).collect();

        // Reset interrupted tasks to pending
        for task in &interrupted {
            checkpoint::update_task_status(&conn, &task.task_id, checkpoint::STATUS_PENDING)
                .map_err(|e| e.to_string())?;
        }

        paths
    }; // MutexGuard dropped here

    // Step 2: Async cleanup of .tmp files (no lock held)
    for dest in &interrupted_dest_paths {
        let tmp_path = crate::copy_engine::atomic_writer::AtomicWriter::temp_path_for(
            Path::new(dest),
        );
        if tmp_path.exists() {
            tokio::fs::remove_file(&tmp_path).await.ok();
        }
    }

    // Step 3: Return updated progress (re-acquires lock)
    get_job_progress(state, job_id)
}

// ─── Volume Commands ──────────────────────────────────────────────────────

/// List all mounted volumes with space information
#[tauri::command]
pub async fn list_volumes() -> Result<CommandResult<Vec<VolumeInfoResponse>>, String> {
    let volumes = volume::list_mounted_volumes()
        .await
        .map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for vol in volumes {
        let usage_percent = if vol.total_bytes > 0 {
            ((vol.total_bytes - vol.available_bytes) as f64 / vol.total_bytes as f64) * 100.0
        } else {
            0.0
        };

        result.push(VolumeInfoResponse {
            id: vol.id,
            name: vol.name,
            mount_point: vol.mount_point.clone(),
            total_bytes: vol.total_bytes,
            available_bytes: vol.available_bytes,
            device_type: vol.device_type.to_string(),
            serial_number: vol.serial_number,
            is_mounted: vol.is_mounted,
            usage_percent,
            is_low: vol.available_bytes < 10_737_418_240,
            is_critical: vol.available_bytes < 1_073_741_824,
        });
    }

    Ok(CommandResult::ok(result))
}

/// Get space info for a specific path
#[tauri::command]
pub fn get_space_info(path: String) -> Result<CommandResult<VolumeSpaceInfo>, String> {
    let p = Path::new(&path);
    match volume::get_volume_space(p) {
        Ok(info) => Ok(CommandResult::ok(info)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

/// Pre-flight check: verify all destinations have enough space
#[tauri::command]
pub fn preflight_check(
    destinations: Vec<(String, u64)>,
) -> Result<CommandResult<Vec<SpaceIssue>>, String> {
    let dest_paths: Vec<(PathBuf, u64)> = destinations
        .into_iter()
        .map(|(p, s)| (PathBuf::from(p), s))
        .collect();

    let issues = volume::preflight_space_check(&dest_paths);

    let result: Vec<SpaceIssue> = issues
        .into_iter()
        .map(|(path, space, required)| SpaceIssue {
            path: path.to_string_lossy().to_string(),
            available_bytes: space.available_bytes,
            required_bytes: required,
            deficit_bytes: required.saturating_sub(space.available_bytes),
        })
        .collect();

    Ok(CommandResult::ok(result))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceIssue {
    pub path: String,
    pub available_bytes: u64,
    pub required_bytes: u64,
    pub deficit_bytes: u64,
}

// ─── Hash Commands ────────────────────────────────────────────────────────

/// Hash a single file with specified algorithms
#[tauri::command]
pub async fn hash_file(
    path: String,
    algorithms: Vec<String>,
) -> Result<CommandResult<Vec<HashResult>>, String> {
    let algos: Vec<HashAlgorithm> = algorithms
        .iter()
        .filter_map(|s| parse_algorithm(s))
        .collect();

    if algos.is_empty() {
        return Ok(CommandResult::err("No valid hash algorithms specified".to_string()));
    }

    let config = HashEngineConfig {
        algorithms: algos,
        buffer_size: 4 * 1024 * 1024,
    };

    match hash_engine::hash_file(Path::new(&path), &config).await {
        Ok(results) => Ok(CommandResult::ok(results)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

// ─── IO Scheduler Commands ────────────────────────────────────────────────

/// Register a device with the IO scheduler
#[tauri::command]
pub fn register_device(
    state: State<'_, AppState>,
    mount_point: String,
    device_type: String,
) -> Result<CommandResult<String>, String> {
    let dt = DeviceType::from_str_loose(&device_type);
    let mut scheduler = state.io_scheduler.lock().map_err(|e| e.to_string())?;
    scheduler.register_device_auto(PathBuf::from(&mount_point), dt);
    Ok(CommandResult::ok(format!("Registered {} as {:?}", mount_point, dt)))
}

/// Get IO scheduler status (returns current device queue states)
#[tauri::command]
pub fn get_scheduler_status(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<DeviceQueueStatusResponse>>, String> {
    let scheduler = state.io_scheduler.lock().map_err(|e| e.to_string())?;

    // Collect status synchronously (avoid holding lock across await)
    let statuses: Vec<DeviceQueueStatusResponse> = scheduler
        .registered_devices()
        .iter()
        .filter_map(|mount| {
            scheduler.get_device_queue(mount).map(|q| DeviceQueueStatusResponse {
                mount_point: mount.to_string_lossy().to_string(),
                device_type: q.config.device_type.to_string(),
                max_concurrent: q.max_concurrent(),
                buffer_size: q.buffer_size(),
            })
        })
        .collect();

    Ok(CommandResult::ok(statuses))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceQueueStatusResponse {
    pub mount_point: String,
    pub device_type: String,
    pub max_concurrent: usize,
    pub buffer_size: usize,
}

// ─── MHL Commands ─────────────────────────────────────────────────────────

/// Create an MHL generation for completed file hashes.
#[tauri::command]
pub async fn create_mhl_generation(
    root_path: String,
    process_type: String,
    file_hashes: HashMap<String, Vec<HashResult>>,
    file_metadata: HashMap<String, (u64, String)>,
) -> Result<CommandResult<String>, String> {
    let root = PathBuf::from(&root_path);
    let mut history = mhl::load_or_create_history(&root)
        .await
        .map_err(|e| e.to_string())?;

    let proc_type = match process_type.as_str() {
        "transfer" => MhlProcessType::Transfer,
        "in-place" => MhlProcessType::InPlace,
        "flatten" => MhlProcessType::Flatten,
        _ => MhlProcessType::Transfer,
    };

    // Convert metadata timestamps
    let metadata: HashMap<String, (u64, chrono::DateTime<chrono::Utc>)> = file_metadata
        .into_iter()
        .map(|(k, (size, ts))| {
            let dt = chrono::DateTime::parse_from_rfc3339(&ts)
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());
            (k, (size, dt))
        })
        .collect();

    let config = MhlConfig::default();

    match mhl::create_generation(&mut history, &file_hashes, &metadata, proc_type, &config).await {
        Ok(path) => Ok(CommandResult::ok(path.to_string_lossy().to_string())),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

/// Verify MHL chain integrity for a directory
#[tauri::command]
pub async fn verify_mhl_chain(
    root_path: String,
) -> Result<CommandResult<Vec<MhlChainVerifyResult>>, String> {
    let root = PathBuf::from(&root_path);
    let history = mhl::load_or_create_history(&root)
        .await
        .map_err(|e| e.to_string())?;

    if history.chain.is_empty() {
        return Ok(CommandResult::err("No MHL history found at this path".to_string()));
    }

    let results = mhl::verify_chain(&history)
        .await
        .map_err(|e| e.to_string())?;

    let response: Vec<MhlChainVerifyResult> = results
        .into_iter()
        .map(|(gen, valid)| MhlChainVerifyResult {
            generation: gen,
            valid,
        })
        .collect();

    Ok(CommandResult::ok(response))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MhlChainVerifyResult {
    pub generation: u32,
    pub valid: bool,
}

// ─── Offload Workflow Commands ────────────────────────────────────────

/// Request to start a full offload workflow
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartOffloadRequest {
    pub name: String,
    pub source_path: String,
    pub dest_paths: Vec<String>,
    pub hash_algorithms: Option<Vec<String>>,
    pub source_verify: Option<bool>,
    pub post_verify: Option<bool>,
    pub generate_mhl: Option<bool>,
}

/// Start an offload workflow. Returns immediately with job_id.
/// Progress is emitted as Tauri events ("offload-event").
#[tauri::command]
pub async fn start_offload(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: StartOffloadRequest,
) -> Result<CommandResult<String>, String> {
    let job_id = uuid::Uuid::new_v4().to_string();

    // Read saved settings as defaults
    let saved = state.settings.lock().map_err(|e| e.to_string())?.clone();

    let algos: Vec<HashAlgorithm> = request
        .hash_algorithms
        .unwrap_or_else(|| saved.hash_algorithms.clone())
        .iter()
        .filter_map(|s| parse_algorithm(s))
        .collect();

    let config = workflow::OffloadConfig {
        job_id: job_id.clone(),
        job_name: request.name,
        source_path: PathBuf::from(&request.source_path),
        dest_paths: request.dest_paths.iter().map(PathBuf::from).collect(),
        hash_algorithms: if algos.is_empty() {
            vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256]
        } else {
            algos
        },
        buffer_size: saved.offload.buffer_size,
        source_verify: request.source_verify.unwrap_or(saved.offload.source_verify),
        post_verify: request.post_verify.unwrap_or(saved.offload.post_verify),
        generate_mhl: request.generate_mhl.unwrap_or(saved.offload.generate_mhl),
        max_retries: saved.offload.max_retries,
    };

    let db = state.db.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    // Spawn the workflow on a background task
    let job_id_for_task = job_id.clone();
    tokio::spawn(async move {
        let wf = workflow::OffloadWorkflow::new(config, db, tx);
        match wf.execute().await {
            Ok(result) => {
                log::info!(
                    "Offload {} completed: {} files, {:.1}s",
                    job_id_for_task,
                    result.total_files,
                    result.duration_secs
                );
            }
            Err(e) => {
                log::error!("Offload {} failed: {}", job_id_for_task, e);
            }
        }
    });

    // Spawn event forwarder: mpsc channel → Tauri events
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            app.emit("offload-event", &event).ok();
        }
    });

    Ok(CommandResult::ok(job_id))
}

// ─── Settings Commands ────────────────────────────────────────────────────

/// Get current application settings
#[tauri::command]
pub fn get_settings(
    state: State<'_, AppState>,
) -> Result<CommandResult<AppSettings>, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(CommandResult::ok(settings.clone()))
}

/// Save application settings to disk
#[tauri::command]
pub fn save_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<CommandResult<bool>, String> {
    // Save to disk
    config::save_settings(&state.app_data_dir, &settings)
        .map_err(|e| e.to_string())?;

    // Update in-memory state
    let mut current = state.settings.lock().map_err(|e| e.to_string())?;
    *current = settings;

    Ok(CommandResult::ok(true))
}

// ─── Helpers ──────────────────────────────────────────────────────────────

fn parse_algorithm(s: &str) -> Option<HashAlgorithm> {
    match s.to_uppercase().as_str() {
        "XXH64" => Some(HashAlgorithm::XXH64),
        "XXH3" => Some(HashAlgorithm::XXH3),
        "XXH128" => Some(HashAlgorithm::XXH128),
        "SHA256" | "SHA-256" => Some(HashAlgorithm::SHA256),
        "MD5" => Some(HashAlgorithm::MD5),
        _ => None,
    }
}

/// Recursively scan a directory and return relative paths with file sizes
async fn scan_directory(root: &Path) -> Result<Vec<(String, u64)>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    let ignore_patterns: Vec<String> = mhl::DEFAULT_IGNORE_PATTERNS
        .iter()
        .map(|s| s.to_string())
        .collect();

    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = entry.metadata().await?;

            // Get relative path
            let rel_path = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            // Check ignore patterns
            if mhl::should_ignore(&rel_path, &ignore_patterns) {
                continue;
            }

            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                files.push((rel_path, metadata.len()));
            }
        }
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}
