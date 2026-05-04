//! Rushes Log — Generate and export shooting day logs.
//!
//! Aggregates job and copy task data into a structured rushes log format
//! for export as CSV, TSV, Excel, PDF, or clipboard copy.
//!
//! Competes with Reel Nice ($29/year) by adding backup status and MHL
//! verification results that no other tool provides.

pub mod excel;
pub mod pdf;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::mhl::{ASCMHL_DIR_NAME, CHAIN_FILE_NAME};

// ─── Data Types ──────────────────────────────────────────────────────────

/// A single entry in the rushes log (one per job/reel)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RushesLogEntry {
    // Job / camera info
    pub job_id: String,
    pub job_name: String,
    pub reel_name: String,
    pub camera_brand: String,
    pub camera_model: String,
    pub clip_count: u32,
    pub first_clip: String,
    pub last_clip: String,
    pub source_path: String,

    // Copy task aggregates
    pub total_size: u64,
    pub total_files: u32,
    pub completed_files: u32,
    pub failed_files: u32,

    // Computed
    pub duration_seconds: f64,
    pub avg_speed_mbps: f64,
    pub backup_status: String, // "Verified" | "Partial" | "Failed" | "Pending"
    pub mhl_verified: bool,
    pub proxy_status: String, // "Generated" | "Partial" | "None"

    // Destinations
    pub dest_paths: Vec<String>,

    // Timestamps
    pub started_at: String,
    pub completed_at: String,

    // Media metadata (from ffprobe, P2)
    pub resolution: Option<String>,
    pub frame_rate: Option<String>,
    pub codec: Option<String>,
    pub color_space: Option<String>,
    pub timecode_range: Option<String>,
    pub thumbnail_path: Option<String>,
}

/// Complete rushes log report for a shooting day
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RushesLogReport {
    pub generated_at: String,
    pub shoot_date: String,
    pub entries: Vec<RushesLogEntry>,
    pub summary: RushesLogSummary,
}

/// Summary statistics for the entire shoot day
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RushesLogSummary {
    pub total_reels: u32,
    pub total_clips: u32,
    pub total_size: u64,
    pub total_duration_seconds: f64,
    pub cameras_used: Vec<String>,
}

// ─── Data Query ──────────────────────────────────────────────────────────

/// Generate a rushes log report for a given date.
pub fn get_rushes_log(conn: &Connection, date: &str) -> Result<RushesLogReport> {
    let mut stmt = conn.prepare(
        "SELECT j.id, j.name, j.source_path, j.status, j.created_at, j.updated_at,
                COALESCE(j.camera_brand, '') as camera_brand,
                COALESCE(j.camera_model, '') as camera_model,
                COALESCE(j.reel_name, '') as reel_name,
                COALESCE(j.clip_count, 0) as clip_count,
                COALESCE(j.first_clip, '') as first_clip,
                COALESCE(j.last_clip, '') as last_clip
         FROM jobs j
         WHERE date(j.created_at) = ?1
         ORDER BY j.created_at ASC",
    )?;

    let jobs: Vec<JobRow> = stmt
        .query_map(rusqlite::params![date], |row| {
            Ok(JobRow {
                id: row.get(0)?,
                name: row.get(1)?,
                source_path: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                camera_brand: row.get(6)?,
                camera_model: row.get(7)?,
                reel_name: row.get(8)?,
                clip_count: row.get(9)?,
                first_clip: row.get(10)?,
                last_clip: row.get(11)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to query jobs for rushes log")?;

    let mut entries = Vec::new();
    let mut summary = RushesLogSummary {
        total_reels: 0,
        total_clips: 0,
        total_size: 0,
        total_duration_seconds: 0.0,
        cameras_used: Vec::new(),
    };

    for job in &jobs {
        let entry = build_entry(conn, job)?;

        summary.total_reels += 1;
        summary.total_clips += entry.clip_count;
        summary.total_size += entry.total_size;
        summary.total_duration_seconds += entry.duration_seconds;

        if !entry.camera_brand.is_empty()
            && entry.camera_brand != "Unknown"
            && !summary.cameras_used.contains(&entry.camera_brand)
        {
            summary.cameras_used.push(entry.camera_brand.clone());
        }

        entries.push(entry);
    }

    Ok(RushesLogReport {
        generated_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        shoot_date: date.to_string(),
        entries,
        summary,
    })
}

/// Internal job row from the database query.
struct JobRow {
    id: String,
    name: String,
    source_path: String,
    status: String,
    created_at: String,
    updated_at: String,
    camera_brand: String,
    camera_model: String,
    reel_name: String,
    clip_count: u32,
    first_clip: String,
    last_clip: String,
}

/// Build a single RushesLogEntry from a job row + aggregated copy_tasks.
fn build_entry(conn: &Connection, job: &JobRow) -> Result<RushesLogEntry> {
    // Aggregate by source file so multi-destination jobs do not double-count
    // files or material size.
    let (total_files_i, completed_files_i, failed_files_i, total_size_i): (i64, i64, i64, i64) =
        conn.query_row(
            "SELECT
                COUNT(*),
                COALESCE(SUM(CASE WHEN failed_count = 0 AND active_count = 0 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN failed_count > 0 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(file_size), 0)
             FROM (
                SELECT source_path,
                       MAX(file_size) AS file_size,
                       SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) AS failed_count,
                       SUM(CASE WHEN status NOT IN ('completed','skipped','failed') THEN 1 ELSE 0 END) AS active_count
                FROM copy_tasks
                WHERE job_id = ?1
                GROUP BY source_path
             )",
            rusqlite::params![job.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    let total_files = total_files_i as u32;
    let completed_files = completed_files_i as u32;
    let failed_files = failed_files_i as u32;
    let total_size = total_size_i.max(0) as u64;

    let mut source_stmt =
        conn.prepare("SELECT source_path FROM copy_tasks WHERE job_id = ?1 GROUP BY source_path ORDER BY source_path ASC")?;
    let source_names: Vec<String> = source_stmt
        .query_map(rusqlite::params![job.id], |row| {
            let source_path: String = row.get(0)?;
            Ok(Path::new(&source_path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string())
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // Get distinct destination root paths
    let mut dest_stmt =
        conn.prepare("SELECT source_path, dest_path FROM copy_tasks WHERE job_id = ?1")?;
    let dest_paths: Vec<String> = dest_stmt
        .query_map(rusqlite::params![job.id], |row| {
            let source_path: String = row.get(0)?;
            let dest_path: String = row.get(1)?;
            Ok(destination_root_from_task(
                &job.source_path,
                &source_path,
                &dest_path,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // Deduplicate dest paths
    let mut unique_dests: Vec<String> = dest_paths
        .into_iter()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    unique_dests.sort();

    // Calculate duration from timestamps
    let duration_seconds = calculate_duration(&job.created_at, &job.updated_at);

    // Calculate average speed
    let avg_speed_mbps = if duration_seconds > 0.0 && total_size > 0 {
        (total_size as f64 / 1_048_576.0) / duration_seconds
    } else {
        0.0
    };

    // Determine backup status
    let backup_status = if total_files == 0 {
        "Pending".to_string()
    } else if failed_files > 0 && completed_files > 0 {
        "Partial".to_string()
    } else if failed_files > 0 {
        "Failed".to_string()
    } else if completed_files == total_files {
        "Verified".to_string()
    } else {
        "Pending".to_string()
    };

    // Check MHL verification status. A completed copy is not enough; every
    // destination root must contain an ASC MHL chain file.
    let mhl_verified = job.status == "completed"
        && failed_files == 0
        && total_files > 0
        && all_destinations_have_mhl_chain(&unique_dests);

    // Query media metadata from the first video task (if available)
    let media_meta: (String, String, String, String, String, String) = conn
        .query_row(
            "SELECT
                COALESCE(resolution, ''),
                COALESCE(frame_rate, ''),
                COALESCE(codec, ''),
                COALESCE(color_space, ''),
                COALESCE(timecode_start, ''),
                COALESCE(thumbnail_path, '')
             FROM copy_tasks
             WHERE job_id = ?1 AND resolution != '' AND resolution IS NOT NULL
             LIMIT 1",
            rusqlite::params![job.id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap_or_default();

    let resolution = if media_meta.0.is_empty() {
        None
    } else {
        Some(media_meta.0)
    };
    let frame_rate = if media_meta.1.is_empty() {
        None
    } else {
        Some(media_meta.1)
    };
    let codec = if media_meta.2.is_empty() {
        None
    } else {
        Some(media_meta.2)
    };
    let color_space = if media_meta.3.is_empty() {
        None
    } else {
        Some(media_meta.3)
    };
    let timecode_range = if media_meta.4.is_empty() {
        None
    } else {
        Some(media_meta.4)
    };
    let thumbnail_path = if media_meta.5.is_empty() {
        None
    } else {
        Some(media_meta.5)
    };

    let video_proxy_sources = query_video_proxy_sources(conn, &job.id).unwrap_or_default();
    let video_files_i = video_proxy_sources.len() as i64;
    let proxy_files_i = video_proxy_sources
        .iter()
        .filter(|(source_path, proxy_path)| {
            !proxy_path.is_empty() || proxy_exists_in_destinations(source_path, &unique_dests)
        })
        .count() as i64;
    let proxy_status = if proxy_files_i == 0 {
        "None".to_string()
    } else if video_files_i == 0 || proxy_files_i >= video_files_i {
        "Generated".to_string()
    } else {
        "Partial".to_string()
    };
    let derived_clip_count = source_names.len() as u32;
    let first_clip = source_names
        .first()
        .cloned()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| job.first_clip.clone());
    let last_clip = source_names
        .last()
        .cloned()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| job.last_clip.clone());

    Ok(RushesLogEntry {
        job_id: job.id.clone(),
        job_name: job.name.clone(),
        reel_name: if job.reel_name.is_empty() {
            // Fallback: use source directory name
            Path::new(&job.source_path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            job.reel_name.clone()
        },
        camera_brand: job.camera_brand.clone(),
        camera_model: job.camera_model.clone(),
        clip_count: if derived_clip_count > 0 {
            derived_clip_count
        } else {
            job.clip_count
        },
        first_clip,
        last_clip,
        source_path: job.source_path.clone(),
        total_size,
        total_files,
        completed_files,
        failed_files,
        duration_seconds,
        avg_speed_mbps,
        backup_status,
        mhl_verified,
        proxy_status,
        dest_paths: unique_dests,
        started_at: job.created_at.clone(),
        completed_at: job.updated_at.clone(),
        resolution,
        frame_rate,
        codec,
        color_space,
        timecode_range,
        thumbnail_path,
    })
}

fn query_video_proxy_sources(conn: &Connection, job_id: &str) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT source_path, COALESCE(MAX(NULLIF(proxy_path, '')), '') as proxy_path
         FROM copy_tasks
         WHERE job_id = ?1
           AND COALESCE(resolution, '') != ''
         GROUP BY source_path
         ORDER BY source_path ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![job_id], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to query video proxy sources")
}

fn proxy_exists_in_destinations(source_path: &str, dest_roots: &[String]) -> bool {
    let source_stem = match Path::new(source_path).file_stem() {
        Some(stem) => stem.to_string_lossy().to_string(),
        None => return false,
    };
    if source_stem.is_empty() {
        return false;
    }

    dest_roots.iter().any(|dest_root| {
        let proxy_dir = Path::new(dest_root).join("Proxies");
        proxy_dir_contains_source_proxy(&proxy_dir, &source_stem)
    })
}

fn proxy_dir_contains_source_proxy(proxy_dir: &Path, source_stem: &str) -> bool {
    let expected_stem = format!("{source_stem}_proxy");
    let Ok(entries) = std::fs::read_dir(proxy_dir) else {
        return false;
    };

    entries.filter_map(std::result::Result::ok).any(|entry| {
        let path = entry.path();
        if !path.is_file() {
            return false;
        }
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("._"))
        {
            return false;
        }
        path.file_stem()
            .is_some_and(|stem| stem.to_string_lossy() == expected_stem)
    })
}

fn destination_root_from_task(job_source_root: &str, source_path: &str, dest_path: &str) -> String {
    let source_root = Path::new(job_source_root);
    let source = Path::new(source_path);
    let dest = Path::new(dest_path);

    if let Ok(rel_path) = source.strip_prefix(source_root) {
        let depth = rel_path.components().count();
        if depth > 0 {
            let mut root = dest.to_path_buf();
            for _ in 0..depth {
                root.pop();
            }
            return root.to_string_lossy().to_string();
        }
    }

    dest.parent()
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .to_string()
}

fn all_destinations_have_mhl_chain(dest_roots: &[String]) -> bool {
    !dest_roots.is_empty()
        && dest_roots.iter().all(|dest| {
            Path::new(dest)
                .join(ASCMHL_DIR_NAME)
                .join(CHAIN_FILE_NAME)
                .is_file()
        })
}

/// Calculate duration in seconds between two SQLite datetime strings.
fn calculate_duration(start: &str, end: &str) -> f64 {
    use chrono::NaiveDateTime;
    let fmt = "%Y-%m-%d %H:%M:%S";
    let start_dt = NaiveDateTime::parse_from_str(start, fmt);
    let end_dt = NaiveDateTime::parse_from_str(end, fmt);

    match (start_dt, end_dt) {
        (Ok(s), Ok(e)) => {
            let duration = e.signed_duration_since(s);
            duration.num_seconds().max(0) as f64
        }
        _ => 0.0,
    }
}

// ─── Export: CSV / TSV ───────────────────────────────────────────────────

/// Export format enum
#[derive(Debug, Clone, PartialEq)]
pub enum ExportFormat {
    Csv,
    Tsv,
}

/// Column headers for the rushes log export
const EXPORT_HEADERS: &[&str] = &[
    "Reel",
    "Camera",
    "Model",
    "Clips",
    "First Clip",
    "Last Clip",
    "Size",
    "Duration",
    "Speed (MB/s)",
    "Status",
    "MHL Verified",
    "Proxy Status",
    "Resolution",
    "Frame Rate",
    "Codec",
    "Color Space",
    "Timecode",
    "Source",
    "Destinations",
    "Start Time",
    "End Time",
];

/// Generate CSV or TSV string from a rushes log report.
pub fn export_to_string(report: &RushesLogReport, format: &ExportFormat) -> String {
    let sep = match format {
        ExportFormat::Csv => ',',
        ExportFormat::Tsv => '\t',
    };

    let mut output = String::new();

    // Header row
    output.push_str(&EXPORT_HEADERS.join(&sep.to_string()));
    output.push('\n');

    // Data rows
    for entry in &report.entries {
        let size_str = format_bytes(entry.total_size);
        let duration_str = format_duration(entry.duration_seconds);
        let speed_str = format!("{:.1}", entry.avg_speed_mbps);
        let mhl_str = if entry.mhl_verified { "Yes" } else { "No" };
        let dests = entry.dest_paths.join("; ");

        let fields = vec![
            escape_field(&entry.reel_name, format),
            escape_field(&entry.camera_brand, format),
            escape_field(&entry.camera_model, format),
            entry.clip_count.to_string(),
            escape_field(&entry.first_clip, format),
            escape_field(&entry.last_clip, format),
            escape_field(&size_str, format),
            escape_field(&duration_str, format),
            speed_str,
            escape_field(&entry.backup_status, format),
            mhl_str.to_string(),
            escape_field(&entry.proxy_status, format),
            escape_field(entry.resolution.as_deref().unwrap_or(""), format),
            escape_field(entry.frame_rate.as_deref().unwrap_or(""), format),
            escape_field(entry.codec.as_deref().unwrap_or(""), format),
            escape_field(entry.color_space.as_deref().unwrap_or(""), format),
            escape_field(entry.timecode_range.as_deref().unwrap_or(""), format),
            escape_field(&entry.source_path, format),
            escape_field(&dests, format),
            escape_field(&entry.started_at, format),
            escape_field(&entry.completed_at, format),
        ];

        output.push_str(&fields.join(&sep.to_string()));
        output.push('\n');
    }

    // Summary row
    output.push('\n');
    output.push_str(&format!(
        "{}Total: {} reels, {} clips, {}",
        sep, // empty first column
        report.summary.total_reels,
        report.summary.total_clips,
        format_bytes(report.summary.total_size),
    ));
    if !report.summary.cameras_used.is_empty() {
        output.push_str(&format!(
            " | Cameras: {}",
            report.summary.cameras_used.join(", ")
        ));
    }
    output.push('\n');

    output
}

/// Save export to a file.
pub fn export_to_file(
    report: &RushesLogReport,
    format: &ExportFormat,
    output_path: &Path,
) -> Result<String> {
    let content = export_to_string(report, format);
    std::fs::write(output_path, &content)
        .with_context(|| format!("Failed to write rushes log to {:?}", output_path))?;
    Ok(output_path.to_string_lossy().to_string())
}

/// Escape a field value for CSV (double-quote if contains comma, quote, or newline).
fn escape_field(value: &str, format: &ExportFormat) -> String {
    match format {
        ExportFormat::Csv => {
            if value.contains(',') || value.contains('"') || value.contains('\n') {
                format!("\"{}\"", value.replace('"', "\"\""))
            } else {
                value.to_string()
            }
        }
        ExportFormat::Tsv => {
            // TSV: replace tabs and newlines
            value.replace(['\t', '\n'], " ")
        }
    }
}

/// Format bytes into human-readable string.
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

/// Format duration in seconds to "Xm Ys" or "Xh Ym" format.
fn format_duration(seconds: f64) -> String {
    let secs = seconds as u64;
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY, name TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                source_path TEXT NOT NULL,
                config_json TEXT,
                camera_brand TEXT DEFAULT '',
                camera_model TEXT DEFAULT '',
                reel_name TEXT DEFAULT '',
                clip_count INTEGER DEFAULT 0,
                first_clip TEXT DEFAULT '',
                last_clip TEXT DEFAULT '',
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
                hash_md5 TEXT, hash_xxh128 TEXT, hash_xxh3 TEXT,
                error_msg TEXT, retry_count INTEGER NOT NULL DEFAULT 0,
                retry_note TEXT,
                resolution TEXT DEFAULT '',
                frame_rate TEXT DEFAULT '',
                codec TEXT DEFAULT '',
                color_space TEXT DEFAULT '',
                bit_depth INTEGER DEFAULT 0,
                timecode_start TEXT DEFAULT '',
                media_duration REAL DEFAULT 0,
                thumbnail_path TEXT DEFAULT '',
                proxy_path TEXT DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .unwrap();
        conn
    }

    fn insert_job(conn: &Connection, id: &str, name: &str, status: &str, brand: &str, reel: &str) {
        conn.execute(
            "INSERT INTO jobs (id, name, status, source_path, camera_brand, reel_name, clip_count, first_clip, last_clip)
             VALUES (?1, ?2, ?3, '/Volumes/CARD_A', ?4, ?5, 3, 'A001.mov', 'A003.mov')",
            rusqlite::params![id, name, status, brand, reel],
        )
        .unwrap();
    }

    fn insert_task(conn: &Connection, job_id: &str, dest: &str, size: u64, status: &str) {
        let tid = uuid::Uuid::new_v4().to_string();
        let source_name = Path::new(dest)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let source_path = format!("/Volumes/CARD_A/{}", source_name);
        conn.execute(
            "INSERT INTO copy_tasks (id, job_id, source_path, dest_path, file_size, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![tid, job_id, source_path, dest, size, status],
        )
        .unwrap();
    }

    fn insert_video_task_with_proxy(
        conn: &Connection,
        job_id: &str,
        source: &str,
        dest: &str,
        proxy: &str,
    ) {
        let tid = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO copy_tasks
                (id, job_id, source_path, dest_path, file_size, status, resolution, proxy_path)
             VALUES (?1, ?2, ?3, ?4, 1000, 'completed', '1920x1080', ?5)",
            rusqlite::params![tid, job_id, source, dest, proxy],
        )
        .unwrap();
    }

    fn write_mhl_chain(dest_root: &Path) {
        let ascmhl_dir = dest_root.join(ASCMHL_DIR_NAME);
        std::fs::create_dir_all(&ascmhl_dir).unwrap();
        std::fs::write(ascmhl_dir.join(CHAIN_FILE_NAME), "<ascmhldirectory/>").unwrap();
    }

    #[test]
    fn test_destination_root_from_nested_task() {
        let root = destination_root_from_task(
            "/Volumes/CARD_A",
            "/Volumes/CARD_A/Clips/A001.mov",
            "/Volumes/SSD1/Clips/A001.mov",
        );
        assert_eq!(root, "/Volumes/SSD1");
    }

    #[test]
    fn test_rushes_log_empty_date() {
        let conn = test_db();
        let report = get_rushes_log(&conn, "2099-01-01").unwrap();
        assert_eq!(report.entries.len(), 0);
        assert_eq!(report.summary.total_reels, 0);
    }

    #[test]
    fn test_rushes_log_with_data() {
        let conn = test_db();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let temp = tempfile::tempdir().unwrap();
        let dst1 = temp.path().join("dst1");
        let dst2 = temp.path().join("dst2");
        write_mhl_chain(&dst1);
        write_mhl_chain(&dst2);

        insert_job(&conn, "j1", "A-Cam Day1", "completed", "ARRI", "A001");
        insert_task(
            &conn,
            "j1",
            &dst1.join("clip.mov").to_string_lossy(),
            1_000_000,
            "completed",
        );
        insert_task(
            &conn,
            "j1",
            &dst2.join("clip.mov").to_string_lossy(),
            1_000_000,
            "completed",
        );

        insert_job(&conn, "j2", "B-Cam Day1", "completed", "RED", "B001");
        insert_task(
            &conn,
            "j2",
            &dst1.join("clip.r3d").to_string_lossy(),
            2_000_000,
            "completed",
        );
        insert_task(
            &conn,
            "j2",
            &dst1.join("clip2.r3d").to_string_lossy(),
            500_000,
            "failed",
        );

        let report = get_rushes_log(&conn, &today).unwrap();
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.summary.total_reels, 2);
        assert_eq!(report.summary.cameras_used.len(), 2);

        // First job: all completed → Verified
        assert_eq!(report.entries[0].backup_status, "Verified");
        assert!(report.entries[0].mhl_verified);
        assert_eq!(report.entries[0].proxy_status, "None");

        // Second job: has failures → Partial
        assert_eq!(report.entries[1].backup_status, "Partial");
        assert!(!report.entries[1].mhl_verified);
    }

    #[test]
    fn test_rushes_log_proxy_status() {
        let conn = test_db();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        insert_job(&conn, "j1", "Proxy Job", "completed", "Generic", "A001");

        insert_video_task_with_proxy(
            &conn,
            "j1",
            "/Volumes/CARD_A/A001.mov",
            "/Volumes/SSD1/A001.mov",
            "/Volumes/SSD1/Proxies/A001_proxy.mp4",
        );
        insert_video_task_with_proxy(
            &conn,
            "j1",
            "/Volumes/CARD_A/A002.mov",
            "/Volumes/SSD1/A002.mov",
            "",
        );

        let report = get_rushes_log(&conn, &today).unwrap();
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].proxy_status, "Partial");
    }

    #[test]
    fn test_rushes_log_detects_existing_proxy_files_without_proxy_path() {
        let conn = test_db();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let temp = tempfile::tempdir().unwrap();
        let dest_root = temp.path().join("Test002");
        let proxy_dir = dest_root.join("Proxies");
        std::fs::create_dir_all(&proxy_dir).unwrap();
        std::fs::write(proxy_dir.join("PS129012_proxy.mp4"), b"proxy").unwrap();
        std::fs::write(proxy_dir.join("._PS129012_proxy.mp4"), b"appledouble").unwrap();

        insert_job(&conn, "j1", "Proxy Job", "completed", "Generic", "A001");
        conn.execute(
            "INSERT INTO copy_tasks
                (id, job_id, source_path, dest_path, file_size, status, resolution, proxy_path)
             VALUES
                (?1, 'j1', '/Volumes/CARD_A/PS129012.MOV', ?2, 1000, 'completed', '3840x2160', '')",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                dest_root.join("PS129012.MOV").to_string_lossy()
            ],
        )
        .unwrap();

        let report = get_rushes_log(&conn, &today).unwrap();
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].proxy_status, "Generated");
    }

    #[test]
    fn test_rushes_log_prefers_task_sources_over_stale_camera_metadata() {
        let conn = test_db();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        conn.execute(
            "INSERT INTO jobs
                (id, name, status, source_path, camera_brand, reel_name, clip_count, first_clip, last_clip)
             VALUES
                ('j1', 'LUMIX Test', 'completed', '/Volumes/LUMIX/TestSource', 'Panasonic', 'TestSource', 10, '._PS128842.MOV', 'PS129015.MOV')",
            [],
        )
        .unwrap();

        for clip in [
            "PS128822.MOV",
            "PS128842.MOV",
            "PS128843.MOV",
            "PS128844.MOV",
            "PS128845.MOV",
            "PS129011.MOV",
            "PS129012.MOV",
            "PS129013.MOV",
            "PS129015.MOV",
        ] {
            let tid = uuid::Uuid::new_v4().to_string();
            let source = format!("/Volumes/LUMIX/TestSource/{clip}");
            let dest = format!("/Volumes/T7S/TestFolder/Test007/{clip}");
            conn.execute(
                "INSERT INTO copy_tasks (id, job_id, source_path, dest_path, file_size, status)
                 VALUES (?1, 'j1', ?2, ?3, 1000, 'completed')",
                rusqlite::params![tid, source, dest],
            )
            .unwrap();
        }

        let report = get_rushes_log(&conn, &today).unwrap();
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].clip_count, 9);
        assert_eq!(report.entries[0].first_clip, "PS128822.MOV");
        assert_eq!(report.entries[0].last_clip, "PS129015.MOV");
    }

    #[test]
    fn test_export_csv() {
        let report = RushesLogReport {
            generated_at: "2026-03-09 10:00:00".to_string(),
            shoot_date: "2026-03-09".to_string(),
            entries: vec![RushesLogEntry {
                job_id: "j1".to_string(),
                job_name: "Test Job".to_string(),
                reel_name: "A001".to_string(),
                camera_brand: "ARRI".to_string(),
                camera_model: String::new(),
                clip_count: 3,
                first_clip: "A001C001.ari".to_string(),
                last_clip: "A001C003.ari".to_string(),
                source_path: "/Volumes/CARD_A".to_string(),
                total_size: 10_737_418_240, // 10 GB
                total_files: 3,
                completed_files: 3,
                failed_files: 0,
                duration_seconds: 120.0,
                avg_speed_mbps: 85.3,
                backup_status: "Verified".to_string(),
                mhl_verified: true,
                proxy_status: "None".to_string(),
                dest_paths: vec!["/Volumes/SSD1".to_string()],
                started_at: "2026-03-09 09:00:00".to_string(),
                completed_at: "2026-03-09 09:02:00".to_string(),
                resolution: Some("4096x2160".to_string()),
                frame_rate: Some("23.976".to_string()),
                codec: Some("Apple ProRes".to_string()),
                color_space: Some("bt709".to_string()),
                timecode_range: Some("01:00:00:00".to_string()),
                thumbnail_path: None,
            }],
            summary: RushesLogSummary {
                total_reels: 1,
                total_clips: 3,
                total_size: 10_737_418_240,
                total_duration_seconds: 120.0,
                cameras_used: vec!["ARRI".to_string()],
            },
        };

        let csv = export_to_string(&report, &ExportFormat::Csv);
        assert!(csv.contains("Reel,Camera,Model"));
        assert!(csv.contains("A001,ARRI,"));
        assert!(csv.contains("10.0 GB"));
        assert!(csv.contains("Verified"));
        assert!(csv.contains("Yes")); // MHL verified
        assert!(csv.contains("4096x2160"));
        assert!(csv.contains("23.976"));
        assert!(csv.contains("Apple ProRes"));

        let tsv = export_to_string(&report, &ExportFormat::Tsv);
        assert!(tsv.contains("Reel\tCamera\tModel"));
        assert!(tsv.contains("A001\tARRI\t"));
    }

    #[test]
    fn test_csv_escaping() {
        assert_eq!(
            escape_field("hello, world", &ExportFormat::Csv),
            "\"hello, world\""
        );
        assert_eq!(
            escape_field("say \"hi\"", &ExportFormat::Csv),
            "\"say \"\"hi\"\"\""
        );
        assert_eq!(escape_field("simple", &ExportFormat::Csv), "simple");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(30.0), "30s");
        assert_eq!(format_duration(90.0), "1m 30s");
        assert_eq!(format_duration(3661.0), "1h 1m");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
    }
}
