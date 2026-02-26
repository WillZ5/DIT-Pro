//! Debug Bundle Export — one-click diagnostic data collection.
//!
//! Creates a JSON bundle containing error logs, recent jobs, system info,
//! and redacted settings for bug reporting.

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::config::AppSettings;
use crate::error_log;
use crate::version::VersionInfo;

// ─── System Info ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub app_version: VersionInfo,
    pub os: String,
    pub arch: String,
    pub cpu_count: usize,
}

fn collect_system_info() -> SystemInfo {
    SystemInfo {
        app_version: VersionInfo::current(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        arch: std::env::consts::ARCH.to_string(),
        cpu_count: std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1),
    }
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

// ─── Recent Jobs ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecentJob {
    id: String,
    name: String,
    status: String,
    source_path: String,
    created_at: String,
}

fn collect_recent_jobs(conn: &Connection) -> Vec<RecentJob> {
    let mut stmt = match conn.prepare(
        "SELECT id, name, status, source_path, created_at FROM jobs ORDER BY created_at DESC LIMIT 20",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = match stmt.query_map([], |row| {
        Ok(RecentJob {
            id: row.get(0)?,
            name: row.get(1)?,
            status: row.get(2)?,
            source_path: row.get(3)?,
            created_at: row.get(4)?,
        })
    }) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    };

    rows
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
    generated_at: String,
    system_info: SystemInfo,
    db_stats: DbStats,
    recent_jobs: Vec<RecentJob>,
    error_log: Vec<error_log::ErrorLogEntry>,
    error_summary: error_log::ErrorLogSummary,
    settings: RedactedSettings,
}

/// Create a debug bundle file in the app data directory.
/// Returns the path to the generated JSON file.
pub fn create_debug_bundle(
    conn: &Connection,
    app_data_dir: &Path,
    settings: &AppSettings,
) -> Result<PathBuf> {
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

    let bundle = DebugBundle {
        generated_at: chrono::Utc::now().to_rfc3339(),
        system_info: collect_system_info(),
        db_stats: collect_db_stats(conn),
        recent_jobs: collect_recent_jobs(conn),
        error_log: error_log_entries,
        error_summary,
        settings: redact_settings(settings),
    };

    // Write to file
    let reports_dir = app_data_dir.join("reports");
    std::fs::create_dir_all(&reports_dir)?;

    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!("dit-debug-{}.json", timestamp);
    let output_path = reports_dir.join(&filename);

    let json = serde_json::to_string_pretty(&bundle)
        .context("Failed to serialize debug bundle")?;
    std::fs::write(&output_path, json)
        .context("Failed to write debug bundle file")?;

    log::info!("Debug bundle exported to {:?}", output_path);
    Ok(output_path)
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
