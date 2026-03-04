//! Debug Bundle Export — comprehensive diagnostic data collection.
//!
//! Creates a detailed JSON bundle containing:
//! - Full version info (semver, git hash, build date, channel)
//! - System environment (OS version, hardware, CPU, memory, disk)
//! - Error logs with structured error codes
//! - Recent job and task history
//! - Redacted settings
//!
//! Designed to provide maximum diagnostic context for bug reports.

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::config::AppSettings;
use crate::error_log;
use crate::version::VersionInfo;

// ─── System Info (Enhanced) ─────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    /// Full application version info
    pub app_version: VersionInfo,
    /// OS name and architecture (e.g., "macos aarch64")
    pub os: String,
    /// CPU architecture
    pub arch: String,
    /// Available CPU cores
    pub cpu_count: usize,
    /// macOS system version (e.g., "15.3.1") or empty
    pub os_version: String,
    /// Hardware model (e.g., "Mac14,2") or empty
    pub hardware_model: String,
    /// Total system memory in bytes
    pub total_memory_bytes: u64,
    /// Hostname (redacted to first 8 chars)
    pub hostname_prefix: String,
    /// Current locale
    pub locale: String,
    /// Tauri version
    pub tauri_version: String,
    /// Rust version used to compile
    pub rust_version: String,
}

fn collect_system_info() -> SystemInfo {
    let os_version = get_os_version().unwrap_or_default();
    let hardware_model = get_hardware_model().unwrap_or_default();
    let total_memory = get_total_memory();
    let hostname = gethostname_safe();
    let hostname_prefix = if hostname.len() > 8 {
        format!("{}…", &hostname[..8])
    } else {
        hostname
    };

    SystemInfo {
        app_version: VersionInfo::current(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        arch: std::env::consts::ARCH.to_string(),
        cpu_count: std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1),
        os_version,
        hardware_model,
        total_memory_bytes: total_memory,
        hostname_prefix,
        locale: std::env::var("LANG").unwrap_or_else(|_| "unknown".into()),
        tauri_version: "2.x".to_string(), // Tauri framework major version
        rust_version: rustc_version_info(),
    }
}

/// Get hostname without external crate (cross-platform)
fn gethostname_safe() -> String {
    // `hostname` command works on macOS, Linux, and Windows
    let output = std::process::Command::new("hostname").output();
    if let Ok(o) = output {
        if o.status.success() {
            return String::from_utf8_lossy(&o.stdout).trim().to_string();
        }
    }
    "unknown".into()
}

/// Get OS version (cross-platform)
fn get_os_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()?;
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }
    #[cfg(windows)]
    {
        // Use `cmd /c ver` to get Windows version string
        let output = std::process::Command::new("cmd")
            .args(["/c", "ver"])
            .output()
            .ok()?;
        if output.status.success() {
            let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !ver.is_empty() {
                return Some(ver);
            }
        }
    }
    None
}

/// Get hardware model identifier (cross-platform)
fn get_hardware_model() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sysctl")
            .args(["-n", "hw.model"])
            .output()
            .ok()?;
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("wmic")
            .args(["computersystem", "get", "model", "/format:value"])
            .output()
            .ok()?;
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some(val) = line.strip_prefix("Model=") {
                    let model = val.trim();
                    if !model.is_empty() {
                        return Some(model.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Get total system memory in bytes (cross-platform)
fn get_total_memory() -> u64 {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output();
        if let Ok(o) = output {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                return s.parse().unwrap_or(0);
            }
        }
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut mem_info = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        unsafe {
            if GlobalMemoryStatusEx(&mut mem_info).is_ok() {
                return mem_info.ullTotalPhys;
            }
        }
    }
    0
}

/// Get rustc version info at runtime
fn rustc_version_info() -> String {
    let output = std::process::Command::new("rustc")
        .arg("--version")
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            return String::from_utf8_lossy(&o.stdout).trim().to_string();
        }
    }
    "unknown".into()
}

// ─── Database Stats ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbStats {
    pub jobs_count: usize,
    pub copy_tasks_count: usize,
    pub error_log_count: usize,
    pub volumes_count: usize,
}

fn collect_db_stats(conn: &Connection) -> DbStats {
    let count = |table: &str| -> usize {
        conn.query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |row| {
            row.get(0)
        })
        .unwrap_or(0)
    };

    DbStats {
        jobs_count: count("jobs"),
        copy_tasks_count: count("copy_tasks"),
        error_log_count: count("error_log"),
        volumes_count: count("volumes"),
    }
}

// ─── Disk Info ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiskInfo {
    mount_point: String,
    total_bytes: u64,
    available_bytes: u64,
    filesystem: String,
}

fn collect_disk_info() -> Vec<DiskInfo> {
    #[cfg(unix)]
    {
        let output = std::process::Command::new("df").args(["-Hl"]).output();
        if let Ok(o) = output {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout);
                // Parse df output — return key mount points
                return text
                    .lines()
                    .skip(1) // skip header
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 6 {
                            let mount = parts.last().unwrap_or(&"").to_string();
                            // Only include relevant mounts (not system volumes)
                            if mount == "/" || mount.starts_with("/Volumes/") {
                                return Some(DiskInfo {
                                    mount_point: mount,
                                    total_bytes: parse_df_size(parts.get(1).unwrap_or(&"0")),
                                    available_bytes: parse_df_size(parts.get(3).unwrap_or(&"0")),
                                    filesystem: parts[0].to_string(),
                                });
                            }
                        }
                        None
                    })
                    .collect();
            }
        }
    }
    #[cfg(windows)]
    {
        // Use the volume module to enumerate drives (reuse existing logic)
        use tokio::runtime::Handle;
        if let Ok(handle) = Handle::try_current() {
            // We're inside an async runtime — use block_in_place
            if let Ok(volumes) = tokio::task::block_in_place(|| {
                handle.block_on(crate::volume::list_mounted_volumes())
            }) {
                return volumes
                    .into_iter()
                    .map(|v| DiskInfo {
                        mount_point: v.mount_point,
                        total_bytes: v.total_bytes,
                        available_bytes: v.available_bytes,
                        filesystem: v.file_system.unwrap_or_default(),
                    })
                    .collect();
            }
        }
        // Fallback: no async runtime available, query C:\ at minimum
        if let Ok(space) = crate::volume::get_volume_space(std::path::Path::new("C:\\")) {
            return vec![DiskInfo {
                mount_point: "C:\\".to_string(),
                total_bytes: space.total_bytes,
                available_bytes: space.available_bytes,
                filesystem: "NTFS".to_string(),
            }];
        }
    }
    Vec::new()
}

fn parse_df_size(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    let last = s.chars().last().unwrap_or('0');
    let num: f64 = s[..s.len() - 1].parse().unwrap_or(0.0);
    match last {
        'T' | 't' => (num * 1_000_000_000_000.0) as u64,
        'G' | 'g' => (num * 1_000_000_000.0) as u64,
        'M' | 'm' => (num * 1_000_000.0) as u64,
        'K' | 'k' => (num * 1_000.0) as u64,
        _ => s.parse().unwrap_or(0),
    }
}

// ─── Recent Jobs ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecentJob {
    id: String,
    name: String,
    status: String,
    source_path: String,
    created_at: String,
    updated_at: String,
    /// Task status breakdown
    task_summary: TaskSummary,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskSummary {
    total: usize,
    completed: usize,
    failed: usize,
    pending: usize,
    copying: usize,
    skipped: usize,
}

fn collect_recent_jobs(conn: &Connection) -> Vec<RecentJob> {
    let mut stmt = match conn.prepare(
        "SELECT id, name, status, source_path, created_at,
                COALESCE(updated_at, created_at) as updated_at
         FROM jobs ORDER BY created_at DESC LIMIT 20",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let jobs: Vec<(String, String, String, String, String, String)> =
        match stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        }) {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(_) => return Vec::new(),
        };

    jobs.into_iter()
        .map(|(id, name, status, source_path, created_at, updated_at)| {
            let task_summary = collect_task_summary(conn, &id);
            RecentJob {
                id,
                name,
                status,
                source_path,
                created_at,
                updated_at,
                task_summary,
            }
        })
        .collect()
}

fn collect_task_summary(conn: &Connection, job_id: &str) -> TaskSummary {
    let mut summary = TaskSummary::default();
    let mut stmt = match conn
        .prepare("SELECT status, COUNT(*) FROM copy_tasks WHERE job_id = ?1 GROUP BY status")
    {
        Ok(s) => s,
        Err(_) => return summary,
    };

    if let Ok(rows) = stmt.query_map(rusqlite::params![job_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
    }) {
        for row in rows.flatten() {
            match row.0.as_str() {
                "completed" => summary.completed = row.1,
                "failed" => summary.failed = row.1,
                "pending" => summary.pending = row.1,
                "copying" | "verifying" => summary.copying += row.1,
                "skipped" => summary.skipped = row.1,
                _ => {}
            }
            summary.total += row.1;
        }
    }
    summary
}

// ─── Recent Failed Tasks (detailed) ─────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailedTask {
    task_id: String,
    job_id: String,
    source_path: String,
    dest_path: String,
    file_size: i64,
    error_msg: String,
    retry_count: i32,
    updated_at: String,
}

fn collect_failed_tasks(conn: &Connection) -> Vec<FailedTask> {
    let mut stmt = match conn.prepare(
        "SELECT id, job_id, source_path, dest_path, file_size,
                COALESCE(error_msg, '') as error_msg,
                retry_count,
                COALESCE(updated_at, '') as updated_at
         FROM copy_tasks WHERE status = 'failed'
         ORDER BY updated_at DESC LIMIT 50",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let result: Vec<FailedTask> = match stmt.query_map([], |row| {
        Ok(FailedTask {
            task_id: row.get(0)?,
            job_id: row.get(1)?,
            source_path: row.get(2)?,
            dest_path: row.get(3)?,
            file_size: row.get(4)?,
            error_msg: row.get(5)?,
            retry_count: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    };
    result
}

// ─── Redacted Settings ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RedactedSettings {
    offload: serde_json::Value,
    io_scheduling: serde_json::Value,
    email_enabled: bool,
    smtp_host: String,
    smtp_port: u16,
    report: serde_json::Value,
}

fn redact_settings(settings: &AppSettings) -> RedactedSettings {
    RedactedSettings {
        offload: serde_json::to_value(&settings.offload).unwrap_or_default(),
        io_scheduling: serde_json::to_value(&settings.io_scheduling).unwrap_or_default(),
        email_enabled: settings.email.enabled,
        smtp_host: settings.email.smtp_host.clone(),
        smtp_port: settings.email.smtp_port,
        report: serde_json::to_value(&settings.report).unwrap_or_default(),
    }
}

// ─── Bundle ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DebugBundle {
    /// Bundle format version for future compatibility
    bundle_version: u32,
    /// UTC timestamp when this bundle was generated
    generated_at: String,
    /// Full system and app version info
    system_info: SystemInfo,
    /// Disk/volume status
    disk_info: Vec<DiskInfo>,
    /// Database table counts
    db_stats: DbStats,
    /// Last 20 jobs with per-job task breakdown
    recent_jobs: Vec<RecentJob>,
    /// Last 50 failed copy tasks with error details
    failed_tasks: Vec<FailedTask>,
    /// Up to 500 recent error log entries
    error_log: Vec<error_log::ErrorLogEntry>,
    /// Error log summary (counts by severity)
    error_summary: error_log::ErrorLogSummary,
    /// Redacted settings (passwords/tokens removed)
    settings: RedactedSettings,
    /// Tauri log file tail (last 200 lines)
    log_tail: Vec<String>,
}

/// Create a debug bundle file in the app data directory.
/// Returns the path to the generated JSON file.
///
/// The bundle includes comprehensive diagnostic data:
/// - Full version info (semver + git hash + build date + channel)
/// - macOS version, hardware model, CPU, memory
/// - Disk/volume space info
/// - Recent job history with per-job task breakdown
/// - Failed task details with error messages
/// - Error log entries (up to 500)
/// - Redacted settings
/// - Tauri log tail
pub fn create_debug_bundle(
    conn: &Connection,
    app_data_dir: &Path,
    settings: &AppSettings,
) -> Result<PathBuf> {
    log::info!("Generating debug bundle...");

    // Collect all diagnostic data
    let error_log_entries = error_log::query_error_log(
        conn,
        &error_log::ErrorLogFilter {
            limit: Some(500),
            ..Default::default()
        },
    )
    .unwrap_or_default();

    let error_summary = error_log::error_log_summary(conn).unwrap_or(error_log::ErrorLogSummary {
        total: 0,
        critical: 0,
        error: 0,
        warning: 0,
        info: 0,
        unresolved: 0,
    });

    let log_tail = collect_log_tail(app_data_dir);

    let bundle = DebugBundle {
        bundle_version: 2,
        generated_at: chrono::Utc::now().to_rfc3339(),
        system_info: collect_system_info(),
        disk_info: collect_disk_info(),
        db_stats: collect_db_stats(conn),
        recent_jobs: collect_recent_jobs(conn),
        failed_tasks: collect_failed_tasks(conn),
        error_log: error_log_entries,
        error_summary,
        settings: redact_settings(settings),
        log_tail,
    };

    // Write to file
    let reports_dir = app_data_dir.join("reports");
    std::fs::create_dir_all(&reports_dir)?;

    let version = VersionInfo::current();
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!("dit-debug-v{}-{}.json", version.full_string, timestamp);
    let output_path = reports_dir.join(&filename);

    let json = serde_json::to_string_pretty(&bundle).context("Failed to serialize debug bundle")?;
    std::fs::write(&output_path, &json).context("Failed to write debug bundle file")?;

    log::info!(
        "Debug bundle exported to {:?} ({} bytes, {} error entries, {} failed tasks)",
        output_path,
        json.len(),
        bundle.error_log.len(),
        bundle.failed_tasks.len()
    );
    Ok(output_path)
}

/// Read the last 200 lines of the Tauri log file
fn collect_log_tail(app_data_dir: &Path) -> Vec<String> {
    // Tauri log plugin writes to app_data_dir/logs/
    let logs_dir = app_data_dir.join("logs");
    if !logs_dir.exists() {
        return vec!["(no log directory found)".into()];
    }

    // Find the most recent .log file
    let mut log_files: Vec<PathBuf> = std::fs::read_dir(&logs_dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "log")
                        .unwrap_or(false)
                })
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();

    log_files.sort_by(|a, b| {
        b.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .cmp(
                &a.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            )
    });

    if let Some(latest_log) = log_files.first() {
        if let Ok(content) = std::fs::read_to_string(latest_log) {
            let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
            let start = if lines.len() > 200 {
                lines.len() - 200
            } else {
                0
            };
            return lines[start..].to_vec();
        }
    }

    vec!["(could not read log file)".into()]
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn test_collect_system_info() {
        let info = collect_system_info();
        assert!(!info.os.is_empty());
        assert!(info.cpu_count > 0);
    }

    #[test]
    fn test_collect_db_stats() {
        let conn = db::init_database(":memory:").unwrap();
        let stats = collect_db_stats(&conn);
        assert_eq!(stats.jobs_count, 0);
        assert_eq!(stats.error_log_count, 0);
    }

    #[test]
    fn test_create_debug_bundle() {
        let conn = db::init_database(":memory:").unwrap();
        let settings = AppSettings::default();
        let tmp = tempfile::tempdir().unwrap();

        let path = create_debug_bundle(&conn, tmp.path(), &settings).unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().contains("dit-debug-"));

        // Verify it's valid JSON
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed.get("systemInfo").is_some());
        assert!(parsed.get("dbStats").is_some());
        assert!(parsed.get("errorSummary").is_some());
    }
}
