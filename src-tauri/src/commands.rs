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
use crate::notify::{self, NotifyEvent};
use crate::preset::{self, WorkflowPreset};
use crate::report::{self, DayReport, JobReport};
use crate::hash_engine::{self, HashAlgorithm, HashEngineConfig, HashResult};
use crate::io_scheduler::IoScheduler;
use crate::mhl::{self, MhlConfig, MhlProcessType};
use crate::volume::{self, DeviceType, VolumeSpaceInfo};
use crate::workflow::{self, CancelToken, PauseToken};

// ─── App State ────────────────────────────────────────────────────────────

/// Handle for controlling a running offload workflow
pub struct WorkflowHandle {
    pub cancel: CancelToken,
    pub pause: PauseToken,
}

/// Shared application state managed by Tauri
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub io_scheduler: Mutex<IoScheduler>,
    pub app_data_dir: PathBuf,
    pub settings: Mutex<AppSettings>,
    pub active_workflows: Arc<Mutex<HashMap<String, WorkflowHandle>>>,
}

// ─── Event Envelope ──────────────────────────────────────────────────────

/// Wraps an OffloadEvent with the job_id so the frontend can demux events
/// from multiple concurrent offload workflows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OffloadEventEnvelope {
    pub job_id: String,
    pub event: workflow::OffloadEvent,
}

// ─── Response Types ───────────────────────────────────────────────────────

/// Standard response for Tauri commands
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult<T: Serialize> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
    /// Structured error info (new in v0.5.0). None for legacy/unstructured errors.
    pub error_info: Option<crate::error::ErrorInfo>,
}

impl<T: Serialize> CommandResult<T> {
    pub fn ok(data: T) -> Self {
        Self { success: true, data: Some(data), error: None, error_info: None }
    }
    pub fn err(msg: String) -> Self {
        Self { success: false, data: None, error: Some(msg), error_info: None }
    }
    /// Create an error result with structured DitError info.
    pub fn err_structured(dit_err: &crate::error::DitError) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(dit_err.to_string()),
            error_info: Some(crate::error::ErrorInfo::from_dit_error(dit_err, None)),
        }
    }

    /// Create an error result with structured DitError AND log the error to the database.
    pub fn err_and_log(
        conn: &Connection,
        dit_err: crate::error::DitError,
        module: &str,
        job_id: Option<&str>,
    ) -> Self {
        let _ = crate::error_log::log_error(conn, &dit_err, module, job_id, None);
        Self::err_structured(&dit_err)
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
    use crate::error::DitError;

    let job_id = uuid::Uuid::new_v4().to_string();
    let source = PathBuf::from(&request.source_path);

    if !source.exists() {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        return Ok(CommandResult::err_and_log(
            &conn,
            DitError::CopySourceNotFound { path: request.source_path.clone() },
            "commands::create_job",
            None,
        ));
    }

    // Scan source directory for files
    let files = scan_directory(&source).await.map_err(|e| e.to_string())?;
    if files.is_empty() {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        return Ok(CommandResult::err_and_log(
            &conn,
            DitError::IoPathNotAccessible {
                path: request.source_path.clone(),
                source: anyhow::anyhow!("No files found in source directory"),
            },
            "commands::create_job",
            None,
        ));
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
                &source_file.to_string_lossy(),
                &dest_file.to_string_lossy(),
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
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for (id, name, source_path, status) in jobs {
        let progress = checkpoint::get_job_progress(&conn, &id).unwrap_or(JobProgress {
            job_id: id.clone(),
            total_tasks: 0,
            completed: 0,
            pending: 0,
            copying: 0,
            failed: 0,
            skipped: 0,
            total_bytes: 0,
            completed_bytes: 0,
        });

        let percent = if progress.total_bytes > 0 {
            (progress.completed_bytes as f64 / progress.total_bytes as f64) * 100.0
        } else {
            0.0
        };

        // Derive effective status from checkpoint progress when DB status is stale
        let effective_status = if status == "pending" && (progress.copying > 0 || (progress.completed > 0 && progress.completed < progress.total_tasks)) {
            "copying".to_string()
        } else if status == "completed" && progress.failed > 0 {
            "completed_with_errors".to_string()
        } else {
            status
        };

        result.push(JobInfo {
            id,
            name,
            status: effective_status,
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

/// Open a path in the system file manager (Finder on macOS)
#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<CommandResult<bool>, String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Ok(CommandResult::err(format!("Path does not exist: {}", path)));
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(CommandResult::ok(true))
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
    pub cascade: Option<bool>,
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
        .unwrap_or_else(|| vec!["XXH64".to_string(), "SHA256".to_string()])
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
        cascade: request.cascade.unwrap_or(saved.offload.cascade),
    };

    let db = state.db.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    // Create cancel/pause tokens and register workflow handle
    let cancel_token = CancelToken::new();
    let pause_token = PauseToken::new();
    {
        let mut workflows = state.active_workflows.lock().map_err(|e| e.to_string())?;
        workflows.insert(job_id.clone(), WorkflowHandle {
            cancel: cancel_token.clone(),
            pause: pause_token.clone_token(),
        });
    }

    // Capture email settings for notification after completion
    let email_settings = state.settings.lock().map_err(|e| e.to_string())?.email.clone();
    let offload_name = config.job_name.clone();
    let notify_app_data_dir = state.app_data_dir.clone();

    let active_workflows = state.active_workflows.clone();
    let db_for_status = state.db.clone();

    // Spawn the workflow on a background task
    let job_id_for_task = job_id.clone();
    tokio::spawn(async move {
        let wf = workflow::OffloadWorkflow::with_cancel_and_pause(config, db, tx, cancel_token, pause_token);
        match wf.execute().await {
            Ok(result) => {
                log::info!(
                    "Offload {} completed: {} files, {:.1}s",
                    job_id_for_task,
                    result.total_files,
                    result.duration_secs
                );

                // Send email notification on completion
                if email_settings.enabled {
                    let event = NotifyEvent::OffloadCompleted {
                        job_id: job_id_for_task.clone(),
                        job_name: offload_name.clone(),
                        file_count: result.total_files,
                        total_bytes: result.total_bytes,
                        duration_secs: result.duration_secs,
                        mhl_generated: !result.mhl_paths.is_empty(),
                        warnings: result.errors.clone(),
                    };
                    if let Err(e) = notify::send_notification(&email_settings, &event, &notify_app_data_dir).await {
                        log::warn!("Failed to send completion notification: {}", e);
                    }
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                // If cancelled, set status to "terminated" instead of "failed"
                if err_msg.contains("cancelled by user") {
                    log::info!("Offload {} terminated by user", job_id_for_task);
                    if let Ok(conn) = db_for_status.lock() {
                        // Log cancellation as a warning (not an error)
                        let dit_err = crate::error::DitError::CopyCancelled;
                        let _ = crate::error_log::log_error(&conn, &dit_err, "workflow::offload", Some(&job_id_for_task), None);
                        checkpoint::update_job_status(&conn, &job_id_for_task, checkpoint::STATUS_TERMINATED).ok();
                    }
                } else {
                    log::error!("Offload {} failed: {}", job_id_for_task, e);

                    // Log the error to error_log with structured DitError
                    if let Ok(conn) = db_for_status.lock() {
                        let dit_err = crate::error::DitError::from(e);
                        let _ = crate::error_log::log_error(&conn, &dit_err, "workflow::offload", Some(&job_id_for_task), None);
                    }

                    // Send email notification on failure
                    if email_settings.enabled {
                        let event = NotifyEvent::OffloadFailed {
                            job_id: job_id_for_task.clone(),
                            job_name: offload_name.clone(),
                            error: err_msg,
                        };
                        if let Err(e) = notify::send_notification(&email_settings, &event, &notify_app_data_dir).await {
                            log::warn!("Failed to send failure notification: {}", e);
                        }
                    }
                }
            }
        }
        // Remove workflow handle on completion
        if let Ok(mut workflows) = active_workflows.lock() {
            workflows.remove(&job_id_for_task);
        }
    });

    // Spawn event forwarder: mpsc channel → Tauri events (wrapped with job_id)
    let job_id_for_events = job_id.clone();
    tokio::spawn(async move {
        // Set tray to active on first event
        crate::tray::update_tray_icon(&app, crate::tray::TrayState::Active);

        while let Some(event) = rx.recv().await {
            // Update tray icon on terminal events
            match &event {
                workflow::OffloadEvent::Complete { .. } => {
                    crate::tray::update_tray_icon(&app, crate::tray::TrayState::Idle);
                }
                workflow::OffloadEvent::Error { .. } => {
                    crate::tray::update_tray_icon(&app, crate::tray::TrayState::Error);
                }
                _ => {}
            }

            let envelope = OffloadEventEnvelope {
                job_id: job_id_for_events.clone(),
                event,
            };
            app.emit("offload-event", &envelope).ok();
        }
    });

    Ok(CommandResult::ok(job_id))
}

/// Resume an interrupted offload workflow. Recovers interrupted tasks and
/// re-launches the copy pipeline for remaining pending tasks.
#[tauri::command]
pub async fn resume_offload(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<CommandResult<String>, String> {
    use crate::error::DitError;

    // Step 1: Read job info
    let (job_name, source_path) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        match conn.query_row(
            "SELECT name, source_path FROM jobs WHERE id = ?1",
            rusqlite::params![job_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        ) {
            Ok(result) => result,
            Err(_) => {
                return Ok(CommandResult::err_and_log(
                    &conn,
                    DitError::DbNotFound { desc: format!("Job {}", job_id) },
                    "commands::resume_offload",
                    Some(&job_id),
                ));
            }
        }
    };

    // Step 2: Recover interrupted tasks (reset to pending, clean .tmp files)
    let interrupted_dest_paths: Vec<String> = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let interrupted = checkpoint::get_interrupted_tasks(&conn, &job_id)
            .map_err(|e| e.to_string())?;
        let paths: Vec<String> = interrupted.iter().map(|t| t.dest_path.clone()).collect();
        for task in &interrupted {
            checkpoint::update_task_status(&conn, &task.task_id, checkpoint::STATUS_PENDING)
                .map_err(|e| e.to_string())?;
        }
        paths
    };

    for dest in &interrupted_dest_paths {
        let tmp_path = crate::copy_engine::atomic_writer::AtomicWriter::temp_path_for(
            Path::new(dest),
        );
        if tmp_path.exists() {
            tokio::fs::remove_file(&tmp_path).await.ok();
        }
    }

    // Step 3: Get all pending tasks and extract dest root paths
    let pending_tasks = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        checkpoint::get_pending_tasks(&conn, &job_id).map_err(|e| e.to_string())?
    };

    if pending_tasks.is_empty() {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        return Ok(CommandResult::err_and_log(
            &conn,
            DitError::DbNotFound { desc: format!("No pending tasks for job {}", job_id) },
            "commands::resume_offload",
            Some(&job_id),
        ));
    }

    // Derive unique destination root paths from task dest_paths relative to source paths
    let dest_roots: Vec<PathBuf> = {
        let mut roots = std::collections::HashSet::new();
        let source_root = PathBuf::from(&source_path);
        for task in &pending_tasks {
            let source_file = PathBuf::from(&task.source_path);
            let dest_file = PathBuf::from(&task.dest_path);
            // dest_root = dest_file without the relative part from source
            if let Ok(rel) = source_file.strip_prefix(&source_root) {
                if let Some(parent) = dest_file.to_str() {
                    let rel_str = rel.to_string_lossy();
                    if parent.ends_with(&*rel_str) {
                        let root = &parent[..parent.len() - rel_str.len()];
                        let root = root.trim_end_matches('/');
                        roots.insert(root.to_string());
                    }
                }
            }
        }
        roots.into_iter().map(PathBuf::from).collect()
    };

    if dest_roots.is_empty() {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        return Ok(CommandResult::err_and_log(
            &conn,
            DitError::CopyDestNotWritable { path: "Could not determine destination paths".to_string() },
            "commands::resume_offload",
            Some(&job_id),
        ));
    }

    // Step 3.5: Validate source and destination paths exist before resuming
    let source_root = Path::new(&source_path);
    if !source_root.exists() {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        return Ok(CommandResult::err_and_log(
            &conn,
            DitError::CopySourceNotFound { path: source_path.clone() },
            "commands::resume_offload",
            Some(&job_id),
        ));
    }

    for dest in &dest_roots {
        if !dest.exists() {
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            return Ok(CommandResult::err_and_log(
                &conn,
                DitError::CopyDestNotWritable { path: dest.to_string_lossy().into() },
                "commands::resume_offload",
                Some(&job_id),
            ));
        }
    }

    // Step 4: Build config from settings + extracted paths
    let saved = state.settings.lock().map_err(|e| e.to_string())?.clone();

    let default_algos = ["XXH64", "SHA256"];
    let algos: Vec<HashAlgorithm> = default_algos
        .iter()
        .filter_map(|s| parse_algorithm(s))
        .collect();

    let config = workflow::OffloadConfig {
        job_id: job_id.clone(),
        job_name: job_name.clone(),
        source_path: PathBuf::from(&source_path),
        dest_paths: dest_roots,
        hash_algorithms: if algos.is_empty() {
            vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256]
        } else {
            algos
        },
        buffer_size: saved.offload.buffer_size,
        source_verify: false, // Skip source verify on resume
        post_verify: true, // Force: interrupted copies MUST be verified after resume
        generate_mhl: saved.offload.generate_mhl,
        max_retries: saved.offload.max_retries,
        cascade: false, // No cascade on resume
    };

    // Step 5: Mark job as copying
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE jobs SET status = 'copying', updated_at = datetime('now') WHERE id = ?1",
            rusqlite::params![job_id],
        )
        .map_err(|e| e.to_string())?;
    }

    let db = state.db.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    // Create cancel/pause tokens and register workflow handle
    let cancel_token = CancelToken::new();
    let pause_token = PauseToken::new();
    {
        let mut workflows = state.active_workflows.lock().map_err(|e| e.to_string())?;
        workflows.insert(job_id.clone(), WorkflowHandle {
            cancel: cancel_token.clone(),
            pause: pause_token.clone_token(),
        });
    }

    let email_settings = state.settings.lock().map_err(|e| e.to_string())?.email.clone();
    let offload_name = job_name.clone();
    let notify_app_data_dir = state.app_data_dir.clone();

    let active_workflows = state.active_workflows.clone();
    let db_for_status = state.db.clone();

    // Step 6: Spawn resume workflow (skips scan & record creation, only processes pending tasks)
    let job_id_for_task = job_id.clone();
    tokio::spawn(async move {
        let wf = workflow::OffloadWorkflow::with_cancel_and_pause(config, db, tx, cancel_token, pause_token);
        match wf.execute_resume().await {
            Ok(result) => {
                log::info!(
                    "Resume offload {} completed: {} files, {:.1}s",
                    job_id_for_task,
                    result.total_files,
                    result.duration_secs
                );

                if email_settings.enabled {
                    let event = NotifyEvent::OffloadCompleted {
                        job_id: job_id_for_task.clone(),
                        job_name: offload_name.clone(),
                        file_count: result.total_files,
                        total_bytes: result.total_bytes,
                        duration_secs: result.duration_secs,
                        mhl_generated: !result.mhl_paths.is_empty(),
                        warnings: result.errors.clone(),
                    };
                    if let Err(e) = notify::send_notification(&email_settings, &event, &notify_app_data_dir).await {
                        log::warn!("Failed to send completion notification: {}", e);
                    }
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                if err_msg.contains("cancelled by user") {
                    log::info!("Resume offload {} terminated by user", job_id_for_task);
                    if let Ok(conn) = db_for_status.lock() {
                        let dit_err = crate::error::DitError::CopyCancelled;
                        let _ = crate::error_log::log_error(&conn, &dit_err, "workflow::resume", Some(&job_id_for_task), None);
                        checkpoint::update_job_status(&conn, &job_id_for_task, checkpoint::STATUS_TERMINATED).ok();
                    }
                } else {
                    log::error!("Resume offload {} failed: {}", job_id_for_task, e);

                    // Log the error to error_log with structured DitError
                    if let Ok(conn) = db_for_status.lock() {
                        let dit_err = crate::error::DitError::from(e);
                        let _ = crate::error_log::log_error(&conn, &dit_err, "workflow::resume", Some(&job_id_for_task), None);
                    }

                    // Send email notification on failure
                    if email_settings.enabled {
                        let event = NotifyEvent::OffloadFailed {
                            job_id: job_id_for_task.clone(),
                            job_name: offload_name.clone(),
                            error: err_msg,
                        };
                        if let Err(e) = notify::send_notification(&email_settings, &event, &notify_app_data_dir).await {
                            log::warn!("Failed to send failure notification: {}", e);
                        }
                    }
                }
            }
        }
        // Remove workflow handle on completion
        if let Ok(mut workflows) = active_workflows.lock() {
            workflows.remove(&job_id_for_task);
        }
    });

    // Spawn event forwarder
    let job_id_for_events = job_id.clone();
    tokio::spawn(async move {
        crate::tray::update_tray_icon(&app, crate::tray::TrayState::Active);

        while let Some(event) = rx.recv().await {
            match &event {
                workflow::OffloadEvent::Complete { .. } => {
                    crate::tray::update_tray_icon(&app, crate::tray::TrayState::Idle);
                }
                workflow::OffloadEvent::Error { .. } => {
                    crate::tray::update_tray_icon(&app, crate::tray::TrayState::Error);
                }
                _ => {}
            }

            let envelope = OffloadEventEnvelope {
                job_id: job_id_for_events.clone(),
                event,
            };
            app.emit("offload-event", &envelope).ok();
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

// ─── Preset Commands ──────────────────────────────────────────────────────

/// List all workflow presets (user + builtin)
#[tauri::command]
pub fn list_presets(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<WorkflowPreset>>, String> {
    let mut store = preset::load_presets(&state.app_data_dir)
        .map_err(|e| e.to_string())?;

    // If no presets exist, seed with built-in defaults
    if store.presets.is_empty() {
        store.presets = preset::builtin_presets();
        preset::save_presets(&state.app_data_dir, &store)
            .map_err(|e| e.to_string())?;
    }

    Ok(CommandResult::ok(store.presets))
}

/// Create a new workflow preset
#[tauri::command]
pub fn create_preset(
    state: State<'_, AppState>,
    preset_data: WorkflowPreset,
) -> Result<CommandResult<WorkflowPreset>, String> {
    match preset::create_preset(&state.app_data_dir, preset_data) {
        Ok(p) => Ok(CommandResult::ok(p)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

/// Update an existing workflow preset
#[tauri::command]
pub fn update_preset(
    state: State<'_, AppState>,
    preset_data: WorkflowPreset,
) -> Result<CommandResult<WorkflowPreset>, String> {
    match preset::update_preset(&state.app_data_dir, preset_data) {
        Ok(p) => Ok(CommandResult::ok(p)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

/// Delete a workflow preset by ID
#[tauri::command]
pub fn delete_preset(
    state: State<'_, AppState>,
    preset_id: String,
) -> Result<CommandResult<bool>, String> {
    match preset::delete_preset(&state.app_data_dir, &preset_id) {
        Ok(_) => Ok(CommandResult::ok(true)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

// ─── Report Commands ──────────────────────────────────────────────────────

/// Get list of dates that have offload jobs (for report date picker)
#[tauri::command]
pub fn get_report_dates(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<String>>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match report::get_report_dates(&conn) {
        Ok(dates) => Ok(CommandResult::ok(dates)),
        Err(e) => Ok(CommandResult::<Vec<String>>::err(e.to_string())),
    }
}

/// Get a day report for a specific date (format: "YYYY-MM-DD")
#[tauri::command]
pub fn get_day_report(
    state: State<'_, AppState>,
    date: String,
) -> Result<CommandResult<DayReport>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match report::get_day_report(&conn, &date) {
        Ok(data) => Ok(CommandResult::ok(data)),
        Err(e) => Ok(CommandResult::<DayReport>::err(e.to_string())),
    }
}

/// Get a detailed job report
#[tauri::command]
pub fn get_job_report(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<CommandResult<JobReport>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match report::get_job_report(&conn, &job_id) {
        Ok(data) => Ok(CommandResult::ok(data)),
        Err(e) => Ok(CommandResult::<JobReport>::err(e.to_string())),
    }
}

/// Generate a day report and save to disk. Returns file path.
/// Supports optional `format` ("html" or "txt") and `output_path` params.
#[tauri::command]
pub fn export_day_report(
    state: State<'_, AppState>,
    date: String,
    format: Option<String>,
    output_path: Option<String>,
) -> Result<CommandResult<String>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let data = report::get_day_report(&conn, &date).map_err(|e: anyhow::Error| e.to_string())?;
    let fmt = format.unwrap_or_else(|| "html".to_string());
    let (content, ext) = if fmt == "txt" {
        (report::render_day_report_txt(&data), "txt")
    } else {
        (report::render_day_report_html(&data), "html")
    };

    if let Some(out) = output_path {
        std::fs::write(&out, &content).map_err(|e| e.to_string())?;
        return Ok(CommandResult::<String>::ok(out));
    }

    let filename = format!("day-report-{}.{}", date, ext);
    match report::save_report(&state.app_data_dir, &filename, &content) {
        Ok(path) => Ok(CommandResult::<String>::ok(path.to_string_lossy().to_string())),
        Err(e) => Ok(CommandResult::<String>::err(e.to_string())),
    }
}

/// Generate a job report and save to disk. Returns file path.
/// Supports optional `format` ("html" or "txt") and `output_path` params.
#[tauri::command]
pub fn export_job_report(
    state: State<'_, AppState>,
    job_id: String,
    format: Option<String>,
    output_path: Option<String>,
) -> Result<CommandResult<String>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let data = report::get_job_report(&conn, &job_id).map_err(|e: anyhow::Error| e.to_string())?;
    let fmt = format.unwrap_or_else(|| "html".to_string());
    let (content, ext) = if fmt == "txt" {
        (report::render_job_report_txt(&data), "txt")
    } else {
        (report::render_job_report_html(&data), "html")
    };

    if let Some(out) = output_path {
        std::fs::write(&out, &content).map_err(|e| e.to_string())?;
        return Ok(CommandResult::<String>::ok(out));
    }

    let filename = format!("job-report-{}.{}", job_id, ext);
    match report::save_report(&state.app_data_dir, &filename, &content) {
        Ok(path) => Ok(CommandResult::<String>::ok(path.to_string_lossy().to_string())),
        Err(e) => Ok(CommandResult::<String>::err(e.to_string())),
    }
}

// ─── Notification Commands ────────────────────────────────────────────────

/// Send a test email to verify SMTP configuration
#[tauri::command]
pub async fn send_test_email(
    state: State<'_, AppState>,
) -> Result<CommandResult<bool>, String> {
    let email_settings = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.email.clone()
    }; // MutexGuard dropped here before any await

    let app_data_dir = state.app_data_dir.clone();
    match notify::send_test_email(&email_settings, &app_data_dir).await {
        Ok(()) => Ok(CommandResult::ok(true)),
        Err(e) => Ok(CommandResult::err(format!("Email test failed: {}", e))),
    }
}

/// Save SMTP password securely (separate from settings to avoid plaintext in JSON)
#[tauri::command]
pub fn save_smtp_password(
    state: State<'_, AppState>,
    password: String,
) -> Result<CommandResult<bool>, String> {
    // Store password in a separate file (not in settings.json)
    let path = state.app_data_dir.join(".smtp_credential");
    std::fs::write(&path, &password).map_err(|e| e.to_string())?;

    // Mark password as set in settings
    let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
    settings.email.smtp_password_set = !password.is_empty();
    config::save_settings(&state.app_data_dir, &settings).map_err(|e| e.to_string())?;

    Ok(CommandResult::ok(true))
}

// ─── Workflow Control Commands ─────────────────────────────────────────────

/// Pause a running offload workflow
#[tauri::command]
pub fn pause_offload(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<CommandResult<bool>, String> {
    let workflows = state.active_workflows.lock().map_err(|e| e.to_string())?;
    if let Some(handle) = workflows.get(&job_id) {
        handle.pause.pause();
        drop(workflows); // release lock before DB access
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        checkpoint::update_job_status(&conn, &job_id, checkpoint::STATUS_PAUSED)
            .map_err(|e| e.to_string())?;
        Ok(CommandResult::ok(true))
    } else {
        Ok(CommandResult::err(format!("No active workflow for job {}", job_id)))
    }
}

/// Resume a paused offload workflow
#[tauri::command]
pub fn resume_paused_offload(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<CommandResult<bool>, String> {
    let workflows = state.active_workflows.lock().map_err(|e| e.to_string())?;
    if let Some(handle) = workflows.get(&job_id) {
        handle.pause.resume();
        drop(workflows);
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        checkpoint::update_job_status(&conn, &job_id, checkpoint::STATUS_COPYING)
            .map_err(|e| e.to_string())?;
        Ok(CommandResult::ok(true))
    } else {
        Ok(CommandResult::err(format!("No active workflow for job {}", job_id)))
    }
}

/// Terminate a running offload workflow (cancel and mark as terminated)
#[tauri::command]
pub fn terminate_offload(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<CommandResult<bool>, String> {
    let workflows = state.active_workflows.lock().map_err(|e| e.to_string())?;
    if let Some(handle) = workflows.get(&job_id) {
        handle.cancel.cancel();
        drop(workflows);
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        checkpoint::update_job_status(&conn, &job_id, checkpoint::STATUS_TERMINATED)
            .map_err(|e| e.to_string())?;
        Ok(CommandResult::ok(true))
    } else {
        Ok(CommandResult::err(format!("No active workflow for job {}", job_id)))
    }
}

/// Pause multiple workflows at once
#[tauri::command]
pub fn batch_pause(
    state: State<'_, AppState>,
    job_ids: Vec<String>,
) -> Result<CommandResult<usize>, String> {
    // Step 1: Acquire workflow lock, pause tokens, collect affected job IDs
    let paused_ids: Vec<String> = {
        let workflows = state.active_workflows.lock().map_err(|e| e.to_string())?;
        let mut ids = Vec::new();
        for job_id in &job_ids {
            if let Some(handle) = workflows.get(job_id) {
                handle.pause.pause();
                ids.push(job_id.clone());
            }
        }
        ids
    }; // workflow lock released

    // Step 2: Acquire DB lock, update statuses
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    for job_id in &paused_ids {
        checkpoint::update_job_status(&conn, job_id, checkpoint::STATUS_PAUSED).ok();
    }

    Ok(CommandResult::ok(paused_ids.len()))
}

/// Terminate multiple workflows at once
#[tauri::command]
pub fn batch_terminate(
    state: State<'_, AppState>,
    job_ids: Vec<String>,
) -> Result<CommandResult<usize>, String> {
    // Step 1: Acquire workflow lock, cancel tokens, collect affected job IDs
    let cancelled_ids: Vec<String> = {
        let workflows = state.active_workflows.lock().map_err(|e| e.to_string())?;
        let mut ids = Vec::new();
        for job_id in &job_ids {
            if let Some(handle) = workflows.get(job_id) {
                handle.cancel.cancel();
                ids.push(job_id.clone());
            }
        }
        ids
    }; // workflow lock released

    // Step 2: Acquire DB lock, update statuses
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    for job_id in &cancelled_ids {
        checkpoint::update_job_status(&conn, job_id, checkpoint::STATUS_TERMINATED).ok();
    }

    Ok(CommandResult::ok(cancelled_ids.len()))
}

/// Clear old job records from the database
#[tauri::command]
pub fn clear_logs(
    state: State<'_, AppState>,
    days: u32,
) -> Result<CommandResult<usize>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match checkpoint::clear_old_jobs(&conn, days) {
        Ok(deleted) => Ok(CommandResult::ok(deleted)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

/// Delete a single job and all its tasks from the database
#[tauri::command]
pub fn delete_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<CommandResult<bool>, String> {
    // Prevent deleting an active workflow
    let workflows = state.active_workflows.lock().map_err(|e| e.to_string())?;
    if workflows.contains_key(&job_id) {
        return Ok(CommandResult::err("Cannot delete an active job. Terminate it first.".to_string()));
    }
    drop(workflows);

    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match checkpoint::delete_job_by_id(&conn, &job_id) {
        Ok(()) => Ok(CommandResult::ok(true)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

/// Delete multiple jobs and all their tasks from the database
#[tauri::command]
pub fn batch_delete(
    state: State<'_, AppState>,
    job_ids: Vec<String>,
) -> Result<CommandResult<usize>, String> {
    let workflows = state.active_workflows.lock().map_err(|e| e.to_string())?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut count = 0;
    for job_id in &job_ids {
        // Skip active workflows
        if workflows.contains_key(job_id) {
            continue;
        }
        if checkpoint::delete_job_by_id(&conn, job_id).is_ok() {
            count += 1;
        }
    }
    Ok(CommandResult::ok(count))
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
            let file_type = entry.file_type().await?;

            // Skip symlinks to avoid infinite loops
            if file_type.is_symlink() {
                continue;
            }

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

            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let metadata = entry.metadata().await?;
                files.push((rel_path, metadata.len()));
            }
        }
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

// ─── Error Log Commands ──────────────────────────────────────────────────────

/// Query error log with optional filters.
#[tauri::command]
pub fn get_error_log(
    state: State<'_, AppState>,
    filter: crate::error_log::ErrorLogFilter,
) -> Result<CommandResult<Vec<crate::error_log::ErrorLogEntry>>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match crate::error_log::query_error_log(&conn, &filter) {
        Ok(entries) => Ok(CommandResult::ok(entries)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

/// Get error log summary counts.
#[tauri::command]
pub fn get_error_log_summary(
    state: State<'_, AppState>,
) -> Result<CommandResult<crate::error_log::ErrorLogSummary>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match crate::error_log::error_log_summary(&conn) {
        Ok(summary) => Ok(CommandResult::ok(summary)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

/// Mark an error log entry as resolved.
#[tauri::command]
pub fn resolve_error_entry(
    state: State<'_, AppState>,
    id: i64,
) -> Result<CommandResult<bool>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match crate::error_log::resolve_error(&conn, id) {
        Ok(resolved) => Ok(CommandResult::ok(resolved)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

/// Clear error log entries. If `older_than_days` is provided, only clear entries older than that.
#[tauri::command]
pub fn clear_error_log_entries(
    state: State<'_, AppState>,
    older_than_days: Option<u32>,
) -> Result<CommandResult<usize>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match crate::error_log::clear_error_log(&conn, older_than_days) {
        Ok(count) => Ok(CommandResult::ok(count)),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}

/// Export a debug bundle (error log + system info + recent jobs) as a JSON file.
#[tauri::command]
pub async fn export_debug_bundle(
    state: State<'_, AppState>,
) -> Result<CommandResult<String>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    let app_data_dir = state.app_data_dir.clone();

    match crate::debug_bundle::create_debug_bundle(&conn, &app_data_dir, &settings) {
        Ok(path) => Ok(CommandResult::ok(path.to_string_lossy().to_string())),
        Err(e) => Ok(CommandResult::err(e.to_string())),
    }
}
