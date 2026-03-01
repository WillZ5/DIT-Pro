//! Report Generation — Shooting day summaries and job reports.
//!
//! Generates HTML reports for:
//! - **Shooting Day Report**: All offload jobs for a given date, files copied, hashes, durations
//! - **Job Detail Report**: Per-job breakdown with per-file hash details
//!
//! Reports are stored in the app data directory under `reports/`.

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── Report Data Types ───────────────────────────────────────────────────

/// Summary of a single completed job for report rendering
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub job_id: String,
    pub job_name: String,
    pub status: String,
    pub source_path: String,
    pub total_files: usize,
    pub completed_files: usize,
    pub failed_files: usize,
    pub total_bytes: u64,
    pub completed_bytes: u64,
    pub created_at: String,
}

/// Per-file copy task detail for detailed reports
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetail {
    pub source_path: String,
    pub dest_path: String,
    pub file_size: u64,
    pub status: String,
    pub hash_xxh64: Option<String>,
    pub hash_sha256: Option<String>,
    pub hash_md5: Option<String>,
    pub hash_xxh128: Option<String>,
    pub hash_xxh3: Option<String>,
    /// Retry history note, e.g. "Round 1 verify failed: hash mismatch; Round 2 retry succeeded"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_note: Option<String>,
}

/// A complete day report
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayReport {
    pub date: String,
    pub jobs: Vec<JobSummary>,
    pub total_files: usize,
    pub total_bytes: u64,
    pub total_jobs: usize,
    pub completed_jobs: usize,
    pub failed_jobs: usize,
}

/// A detailed job report
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobReport {
    pub summary: JobSummary,
    pub tasks: Vec<TaskDetail>,
    pub dest_paths: Vec<String>,
}

// ─── Report Generation (Data) ─────────────────────────────────────────────

/// Query all jobs for a given date (format: "YYYY-MM-DD").
pub fn get_day_report(conn: &Connection, date: &str) -> Result<DayReport> {
    let mut stmt = conn.prepare(
        "SELECT id, name, status, source_path, created_at
         FROM jobs
         WHERE date(created_at) = ?1
         ORDER BY created_at ASC",
    )?;

    let jobs_raw: Vec<(String, String, String, String, String)> = stmt
        .query_map(rusqlite::params![date], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to read job records for day report")?;

    let mut jobs = Vec::new();
    let mut total_files = 0usize;
    let mut total_bytes = 0u64;
    let mut completed_jobs = 0usize;
    let mut failed_jobs = 0usize;

    for (id, name, status, source_path, created_at) in &jobs_raw {
        let (t_files, c_files, f_files, t_bytes, c_bytes) = query_job_stats(conn, id)?;

        total_files += t_files;
        total_bytes += t_bytes;

        if status == "completed" {
            completed_jobs += 1;
        } else if status.contains("error") || status == "failed" {
            failed_jobs += 1;
        }

        jobs.push(JobSummary {
            job_id: id.clone(),
            job_name: name.clone(),
            status: status.clone(),
            source_path: source_path.clone(),
            total_files: t_files,
            completed_files: c_files,
            failed_files: f_files,
            total_bytes: t_bytes,
            completed_bytes: c_bytes,
            created_at: created_at.clone(),
        });
    }

    Ok(DayReport {
        date: date.to_string(),
        jobs,
        total_files,
        total_bytes,
        total_jobs: jobs_raw.len(),
        completed_jobs,
        failed_jobs,
    })
}

/// Query detailed report for a single job.
pub fn get_job_report(conn: &Connection, job_id: &str) -> Result<JobReport> {
    let (name, status, source_path, created_at): (String, String, String, String) = conn
        .query_row(
            "SELECT name, status, source_path, created_at FROM jobs WHERE id = ?1",
            rusqlite::params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .with_context(|| format!("Job not found: {}", job_id))?;

    let (t_files, c_files, f_files, t_bytes, c_bytes) = query_job_stats(conn, job_id)?;

    let summary = JobSummary {
        job_id: job_id.to_string(),
        job_name: name,
        status,
        source_path,
        total_files: t_files,
        completed_files: c_files,
        failed_files: f_files,
        total_bytes: t_bytes,
        completed_bytes: c_bytes,
        created_at,
    };

    // Get all task details
    let mut stmt = conn.prepare(
        "SELECT source_path, dest_path, file_size, status,
                hash_xxh64, hash_sha256, hash_md5, hash_xxh128, hash_xxh3,
                retry_note
         FROM copy_tasks WHERE job_id = ?1 ORDER BY source_path ASC",
    )?;

    let tasks: Vec<TaskDetail> = stmt
        .query_map(rusqlite::params![job_id], |row| {
            Ok(TaskDetail {
                source_path: row.get(0)?,
                dest_path: row.get(1)?,
                file_size: row.get(2)?,
                status: row.get(3)?,
                hash_xxh64: row.get(4)?,
                hash_sha256: row.get(5)?,
                hash_md5: row.get(6)?,
                hash_xxh128: row.get(7)?,
                hash_xxh3: row.get(8)?,
                retry_note: row.get(9)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to read task details for job report")?;

    // Distinct destinations
    let dest_paths: Vec<String> = tasks
        .iter()
        .map(|t| {
            Path::new(&t.dest_path)
                .parent()
                .unwrap_or(Path::new(""))
                .to_string_lossy()
                .to_string()
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    Ok(JobReport {
        summary,
        tasks,
        dest_paths,
    })
}

/// Get all dates that have jobs (for date picker in frontend).
pub fn get_report_dates(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT date(created_at) as d FROM jobs ORDER BY d DESC")?;
    let dates: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to read report dates")?;
    Ok(dates)
}

// ─── HTML Rendering ───────────────────────────────────────────────────────

/// Generate an HTML day report string.
pub fn render_day_report_html(report: &DayReport) -> String {
    let mut html = String::new();
    html.push_str(&format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>DIT Pro — Day Report {date}</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
         max-width: 900px; margin: 40px auto; padding: 0 20px; background: #0a0a1a; color: #e0e0e0; }}
  h1 {{ color: #fff; border-bottom: 2px solid #2196f3; padding-bottom: 10px; }}
  h2 {{ color: #2196f3; margin-top: 30px; }}
  .summary {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 16px; margin: 20px 0; }}
  .stat {{ background: #1a1a2e; border: 1px solid #333; border-radius: 8px; padding: 16px; text-align: center; }}
  .stat-value {{ font-size: 28px; font-weight: 700; color: #fff; }}
  .stat-label {{ font-size: 12px; color: #888; text-transform: uppercase; margin-top: 4px; }}
  table {{ width: 100%; border-collapse: collapse; margin: 16px 0; }}
  th, td {{ padding: 10px 12px; text-align: left; border-bottom: 1px solid #333; }}
  th {{ background: #1a1a2e; color: #888; font-size: 11px; text-transform: uppercase; }}
  td {{ font-size: 13px; }}
  .status-completed {{ color: #4caf50; }}
  .status-failed {{ color: #f44336; }}
  .status-pending {{ color: #ff9800; }}
  .footer {{ margin-top: 40px; padding-top: 20px; border-top: 1px solid #333; font-size: 11px; color: #666; }}
</style>
</head>
<body>
<h1>Shooting Day Report — {date}</h1>
<div class="summary">
  <div class="stat"><div class="stat-value">{total_jobs}</div><div class="stat-label">Total Jobs</div></div>
  <div class="stat"><div class="stat-value">{total_files}</div><div class="stat-label">Total Files</div></div>
  <div class="stat"><div class="stat-value">{total_size}</div><div class="stat-label">Total Data</div></div>
  <div class="stat"><div class="stat-value">{completed_jobs}/{total_jobs}</div><div class="stat-label">Completed</div></div>
</div>
"#,
        date = report.date,
        total_jobs = report.total_jobs,
        total_files = report.total_files,
        total_size = format_bytes(report.total_bytes),
        completed_jobs = report.completed_jobs,
    ));

    if !report.jobs.is_empty() {
        html.push_str("<h2>Jobs</h2>\n<table>\n");
        html.push_str("<tr><th>Name</th><th>Status</th><th>Source</th><th>Files</th><th>Size</th><th>Time</th></tr>\n");
        for job in &report.jobs {
            let status_class = match job.status.as_str() {
                "completed" => "status-completed",
                s if s.contains("error") || s == "failed" => "status-failed",
                _ => "status-pending",
            };
            let files = format!("{}/{}", job.completed_files, job.total_files);
            let size = format_bytes(job.total_bytes);
            html.push_str(&format!(
                "<tr><td>{name}</td><td class=\"{status_class}\">{status}</td><td>{source}</td><td>{files}</td><td>{size}</td><td>{time}</td></tr>\n",
                name = html_escape(&job.job_name),
                status_class = status_class,
                status = html_escape(&job.status),
                source = html_escape(&job.source_path),
                time = html_escape(&job.created_at),
            ));
        }
        html.push_str("</table>\n");
    }

    html.push_str(&format!(
        "<div class=\"footer\">Generated by DIT Pro at {}</div>\n</body>\n</html>",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));

    html
}

/// Generate an HTML report for a single job with task-level detail.
pub fn render_job_report_html(report: &JobReport) -> String {
    let s = &report.summary;
    let mut html = String::new();
    html.push_str(&format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>DIT Pro — Job Report: {name}</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
         max-width: 1000px; margin: 40px auto; padding: 0 20px; background: #0a0a1a; color: #e0e0e0; }}
  h1 {{ color: #fff; border-bottom: 2px solid #2196f3; padding-bottom: 10px; }}
  h2 {{ color: #2196f3; margin-top: 30px; }}
  .meta {{ font-size: 13px; color: #888; margin: 8px 0; }}
  .summary {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 16px; margin: 20px 0; }}
  .stat {{ background: #1a1a2e; border: 1px solid #333; border-radius: 8px; padding: 16px; text-align: center; }}
  .stat-value {{ font-size: 28px; font-weight: 700; color: #fff; }}
  .stat-label {{ font-size: 12px; color: #888; text-transform: uppercase; margin-top: 4px; }}
  table {{ width: 100%; border-collapse: collapse; margin: 16px 0; font-size: 12px; }}
  th, td {{ padding: 8px 10px; text-align: left; border-bottom: 1px solid #333; }}
  th {{ background: #1a1a2e; color: #888; font-size: 10px; text-transform: uppercase; }}
  .status-completed {{ color: #4caf50; }}
  .status-failed {{ color: #f44336; }}
  .hash {{ font-family: 'SF Mono', Monaco, Consolas, monospace; font-size: 10px; color: #9c27b0; word-break: break-all; }}
  .retry-note-row td {{ padding: 2px 10px 6px 20px; border-bottom: 1px solid #222; background: #0f0f1f; }}
  .retry-note {{ font-size: 11px; color: #ff9800; font-style: italic; }}
  .footer {{ margin-top: 40px; padding-top: 20px; border-top: 1px solid #333; font-size: 11px; color: #666; }}
</style>
</head>
<body>
<h1>Job Report: {name}</h1>
<p class="meta">Source: {source} &nbsp;|&nbsp; Created: {created}</p>
<div class="summary">
  <div class="stat"><div class="stat-value">{total_files}</div><div class="stat-label">Total Files</div></div>
  <div class="stat"><div class="stat-value">{completed_files}</div><div class="stat-label">Completed</div></div>
  <div class="stat"><div class="stat-value">{failed_files}</div><div class="stat-label">Failed</div></div>
  <div class="stat"><div class="stat-value">{total_size}</div><div class="stat-label">Total Data</div></div>
</div>
"#,
        name = html_escape(&s.job_name),
        source = html_escape(&s.source_path),
        created = html_escape(&s.created_at),
        total_files = s.total_files,
        completed_files = s.completed_files,
        failed_files = s.failed_files,
        total_size = format_bytes(s.total_bytes),
    ));

    // Detect which hash algorithms have data (dynamic columns)
    let has_xxh64 = report.tasks.iter().any(|t| t.hash_xxh64.is_some());
    let has_sha256 = report.tasks.iter().any(|t| t.hash_sha256.is_some());
    let has_md5 = report.tasks.iter().any(|t| t.hash_md5.is_some());
    let has_xxh128 = report.tasks.iter().any(|t| t.hash_xxh128.is_some());
    let has_xxh3 = report.tasks.iter().any(|t| t.hash_xxh3.is_some());

    // Task table
    html.push_str("<h2>File Details</h2>\n<table>\n");
    html.push_str("<tr><th>Source</th><th>Destination</th><th>Size</th><th>Status</th>");
    if has_xxh64 {
        html.push_str("<th>XXH64</th>");
    }
    if has_sha256 {
        html.push_str("<th>SHA-256</th>");
    }
    if has_md5 {
        html.push_str("<th>MD5</th>");
    }
    if has_xxh128 {
        html.push_str("<th>XXH128</th>");
    }
    if has_xxh3 {
        html.push_str("<th>XXH3</th>");
    }
    html.push_str("</tr>\n");

    for task in &report.tasks {
        let status_class = if task.status == "completed" {
            "status-completed"
        } else {
            "status-failed"
        };

        // Show just filename for source/dest to keep table readable
        let src_name = Path::new(&task.source_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let dest_short = Path::new(&task.dest_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();

        html.push_str(&format!(
            "<tr><td title=\"{src_full}\">{src_name}</td>\
             <td title=\"{dest_full}\">{dest_short}</td>\
             <td>{size}</td>\
             <td class=\"{status_class}\">{status}</td>",
            src_full = html_escape(&task.source_path),
            src_name = html_escape(&src_name),
            dest_full = html_escape(&task.dest_path),
            dest_short = html_escape(&dest_short),
            size = format_bytes(task.file_size),
            status_class = status_class,
            status = html_escape(&task.status),
        ));
        if has_xxh64 {
            html.push_str(&format!(
                "<td class=\"hash\">{}</td>",
                task.hash_xxh64.as_deref().unwrap_or("—")
            ));
        }
        if has_sha256 {
            html.push_str(&format!(
                "<td class=\"hash\">{}</td>",
                task.hash_sha256
                    .as_deref()
                    .map(|s| &s[..16.min(s.len())])
                    .unwrap_or("—")
            ));
        }
        if has_md5 {
            html.push_str(&format!(
                "<td class=\"hash\">{}</td>",
                task.hash_md5.as_deref().unwrap_or("—")
            ));
        }
        if has_xxh128 {
            html.push_str(&format!(
                "<td class=\"hash\">{}</td>",
                task.hash_xxh128
                    .as_deref()
                    .map(|s| &s[..16.min(s.len())])
                    .unwrap_or("—")
            ));
        }
        if has_xxh3 {
            html.push_str(&format!(
                "<td class=\"hash\">{}</td>",
                task.hash_xxh3
                    .as_deref()
                    .map(|s| &s[..16.min(s.len())])
                    .unwrap_or("—")
            ));
        }
        html.push_str("</tr>\n");

        // Show retry note below the file row if present
        if let Some(ref note) = task.retry_note {
            let col_count = 4
                + has_xxh64 as usize
                + has_sha256 as usize
                + has_md5 as usize
                + has_xxh128 as usize
                + has_xxh3 as usize;
            html.push_str(&format!(
                "<tr class=\"retry-note-row\"><td colspan=\"{}\">\
                 <span class=\"retry-note\">{}</span></td></tr>\n",
                col_count,
                html_escape(note),
            ));
        }
    }
    html.push_str("</table>\n");

    html.push_str(&format!(
        "<div class=\"footer\">Generated by DIT Pro at {}</div>\n</body>\n</html>",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));

    html
}

// ─── TXT Rendering ────────────────────────────────────────────────────────

/// Generate a plain-text day report string (aligned text table).
pub fn render_day_report_txt(report: &DayReport) -> String {
    let mut txt = String::new();
    txt.push_str(&format!("DIT Pro — Shooting Day Report: {}\n", report.date));
    txt.push_str(&"=".repeat(72));
    txt.push('\n');
    txt.push_str(&format!(
        "Total Jobs: {}  |  Total Files: {}  |  Total Data: {}  |  Completed: {}/{}\n\n",
        report.total_jobs,
        report.total_files,
        format_bytes(report.total_bytes),
        report.completed_jobs,
        report.total_jobs,
    ));

    if !report.jobs.is_empty() {
        txt.push_str(&format!(
            "{:<30} {:<14} {:<20} {:>8} {:>10}\n",
            "Name", "Status", "Source", "Files", "Size"
        ));
        txt.push_str(&"-".repeat(72));
        txt.push('\n');
        for job in &report.jobs {
            txt.push_str(&format!(
                "{:<30} {:<14} {:<20} {:>8} {:>10}\n",
                truncate_str(&job.job_name, 29),
                &job.status,
                truncate_str(&job.source_path, 19),
                format!("{}/{}", job.completed_files, job.total_files),
                format_bytes(job.total_bytes),
            ));
        }
    }

    txt.push_str(&format!(
        "\nGenerated by DIT Pro at {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));
    txt
}

/// Generate a plain-text job report string with task-level detail.
pub fn render_job_report_txt(report: &JobReport) -> String {
    let s = &report.summary;
    let mut txt = String::new();
    txt.push_str(&format!("DIT Pro — Job Report: {}\n", s.job_name));
    txt.push_str(&"=".repeat(90));
    txt.push('\n');
    txt.push_str(&format!("Source:  {}\n", s.source_path));
    txt.push_str(&format!("Created: {}\n", s.created_at));
    txt.push_str(&format!(
        "Files: {} total, {} completed, {} failed  |  Data: {}\n\n",
        s.total_files,
        s.completed_files,
        s.failed_files,
        format_bytes(s.total_bytes),
    ));

    txt.push_str(&format!(
        "{:<30} {:<30} {:>10} {:<10} {:<16}\n",
        "Source", "Destination", "Size", "Status", "XXH64"
    ));
    txt.push_str(&"-".repeat(90));
    txt.push('\n');

    for task in &report.tasks {
        let src_name = Path::new(&task.source_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let dest_name = Path::new(&task.dest_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        txt.push_str(&format!(
            "{:<30} {:<30} {:>10} {:<10} {:<16}\n",
            truncate_str(&src_name, 29),
            truncate_str(&dest_name, 29),
            format_bytes(task.file_size),
            &task.status,
            task.hash_xxh64.as_deref().unwrap_or("—"),
        ));
        if let Some(ref note) = task.retry_note {
            txt.push_str(&format!("    >> {}\n", note));
        }
    }

    txt.push_str(&format!(
        "\nGenerated by DIT Pro at {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));
    txt
}

/// Truncate a string to a maximum display length (in chars), appending "…" if truncated.
/// Safe for multi-byte UTF-8 characters (CJK, emoji, etc).
fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end = max.saturating_sub(1);
        let truncated: String = s.chars().take(end).collect();
        format!("{}…", truncated)
    }
}

/// Save an HTML report to disk, returning the file path.
pub fn save_report(app_data_dir: &Path, filename: &str, html: &str) -> Result<PathBuf> {
    let report_dir = app_data_dir.join("reports");
    std::fs::create_dir_all(&report_dir)
        .with_context(|| format!("Cannot create reports directory: {:?}", report_dir))?;

    let path = report_dir.join(filename);
    std::fs::write(&path, html).with_context(|| format!("Failed to write report: {:?}", path))?;

    Ok(path)
}

// ─── Helpers ──────────────────────────────────────────────────────────────

fn query_job_stats(conn: &Connection, job_id: &str) -> Result<(usize, usize, usize, u64, u64)> {
    let total_files: usize = conn.query_row(
        "SELECT COUNT(*) FROM copy_tasks WHERE job_id = ?1",
        rusqlite::params![job_id],
        |row| row.get(0),
    )?;

    let completed_files: usize = conn.query_row(
        "SELECT COUNT(*) FROM copy_tasks WHERE job_id = ?1 AND status = 'completed'",
        rusqlite::params![job_id],
        |row| row.get(0),
    )?;

    let failed_files: usize = conn.query_row(
        "SELECT COUNT(*) FROM copy_tasks WHERE job_id = ?1 AND status = 'failed'",
        rusqlite::params![job_id],
        |row| row.get(0),
    )?;

    let total_bytes: u64 = conn.query_row(
        "SELECT COALESCE(SUM(file_size), 0) FROM copy_tasks WHERE job_id = ?1",
        rusqlite::params![job_id],
        |row| row.get(0),
    )?;

    let completed_bytes: u64 = conn.query_row(
        "SELECT COALESCE(SUM(file_size), 0) FROM copy_tasks WHERE job_id = ?1 AND status = 'completed'",
        rusqlite::params![job_id],
        |row| row.get(0),
    )?;

    Ok((
        total_files,
        completed_files,
        failed_files,
        total_bytes,
        completed_bytes,
    ))
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

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
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
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .unwrap();
        conn
    }

    fn insert_test_job(conn: &Connection, id: &str, name: &str, status: &str) {
        conn.execute(
            "INSERT INTO jobs (id, name, status, source_path, created_at)
             VALUES (?1, ?2, ?3, '/Volumes/CARD_A', datetime('now'))",
            rusqlite::params![id, name, status],
        )
        .unwrap();
    }

    fn insert_test_task(
        conn: &Connection,
        job_id: &str,
        source: &str,
        dest: &str,
        size: u64,
        status: &str,
    ) {
        let tid = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO copy_tasks (id, job_id, source_path, dest_path, file_size, status,
             hash_xxh64, hash_sha256)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'abc123', 'def456abcdef7890')",
            rusqlite::params![tid, job_id, source, dest, size, status],
        )
        .unwrap();
    }

    #[test]
    fn test_day_report_empty() {
        let conn = test_db();
        let report = get_day_report(&conn, "2026-01-01").unwrap();
        assert_eq!(report.total_jobs, 0);
        assert!(report.jobs.is_empty());
    }

    #[test]
    fn test_day_report_with_jobs() {
        let conn = test_db();
        insert_test_job(&conn, "job-1", "Day 1 A-Cam", "completed");
        insert_test_task(
            &conn,
            "job-1",
            "/src/A001.mov",
            "/dst/A001.mov",
            1000,
            "completed",
        );
        insert_test_task(
            &conn,
            "job-1",
            "/src/A002.mov",
            "/dst/A002.mov",
            2000,
            "completed",
        );

        insert_test_job(&conn, "job-2", "Day 1 B-Cam", "completed_with_errors");
        insert_test_task(
            &conn,
            "job-2",
            "/src/B001.mov",
            "/dst/B001.mov",
            500,
            "completed",
        );
        insert_test_task(
            &conn,
            "job-2",
            "/src/B002.mov",
            "/dst/B002.mov",
            500,
            "failed",
        );

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let report = get_day_report(&conn, &today).unwrap();

        assert_eq!(report.total_jobs, 2);
        assert_eq!(report.total_files, 4);
        assert_eq!(report.total_bytes, 4000);
        assert_eq!(report.completed_jobs, 1); // only "completed" status counts
        assert_eq!(report.jobs[0].job_name, "Day 1 A-Cam");
    }

    #[test]
    fn test_job_report_detail() {
        let conn = test_db();
        insert_test_job(&conn, "job-detail", "Detail Test", "completed");
        insert_test_task(
            &conn,
            "job-detail",
            "/src/clip.mov",
            "/dst1/clip.mov",
            5000,
            "completed",
        );
        insert_test_task(
            &conn,
            "job-detail",
            "/src/clip.mov",
            "/dst2/clip.mov",
            5000,
            "completed",
        );

        let report = get_job_report(&conn, "job-detail").unwrap();
        assert_eq!(report.summary.total_files, 2);
        assert_eq!(report.summary.completed_files, 2);
        assert_eq!(report.tasks.len(), 2);
        assert!(report.tasks[0].hash_xxh64.is_some());
    }

    #[test]
    fn test_report_dates() {
        let conn = test_db();
        insert_test_job(&conn, "j1", "Job1", "completed");
        insert_test_job(&conn, "j2", "Job2", "completed");

        let dates = get_report_dates(&conn).unwrap();
        assert!(!dates.is_empty());
    }

    #[test]
    fn test_render_day_report_html() {
        let report = DayReport {
            date: "2026-02-25".to_string(),
            jobs: vec![JobSummary {
                job_id: "j1".to_string(),
                job_name: "Day 1 <A-Cam>".to_string(), // test escaping
                status: "completed".to_string(),
                source_path: "/Volumes/CARD".to_string(),
                total_files: 10,
                completed_files: 10,
                failed_files: 0,
                total_bytes: 1073741824,
                completed_bytes: 1073741824,
                created_at: "2026-02-25 09:00:00".to_string(),
            }],
            total_files: 10,
            total_bytes: 1073741824,
            total_jobs: 1,
            completed_jobs: 1,
            failed_jobs: 0,
        };

        let html = render_day_report_html(&report);
        assert!(html.contains("Day Report"));
        assert!(html.contains("2026-02-25"));
        assert!(html.contains("1.0 GB"));
        assert!(html.contains("&lt;A-Cam&gt;")); // escaped
        assert!(html.contains("status-completed"));
    }

    #[test]
    fn test_render_job_report_html() {
        let report = JobReport {
            summary: JobSummary {
                job_id: "j1".to_string(),
                job_name: "Test Job".to_string(),
                status: "completed".to_string(),
                source_path: "/Volumes/CARD".to_string(),
                total_files: 2,
                completed_files: 2,
                failed_files: 0,
                total_bytes: 2048,
                completed_bytes: 2048,
                created_at: "2026-02-25 09:00:00".to_string(),
            },
            tasks: vec![TaskDetail {
                source_path: "/Volumes/CARD/clip.mov".to_string(),
                dest_path: "/Volumes/SSD/clip.mov".to_string(),
                file_size: 1024,
                status: "completed".to_string(),
                hash_xxh64: Some("abc123def456".to_string()),
                hash_sha256: Some("0123456789abcdef0123456789abcdef".to_string()),
                hash_md5: None,
                hash_xxh128: None,
                hash_xxh3: None,
                retry_note: None,
            }],
            dest_paths: vec!["/Volumes/SSD".to_string()],
        };

        let html = render_job_report_html(&report);
        assert!(html.contains("Test Job"));
        assert!(html.contains("abc123def456"));
        assert!(html.contains("clip.mov"));
    }

    #[test]
    fn test_save_report() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = save_report(tmp.path(), "test-report.html", "<html>test</html>").unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "<html>test</html>");
    }
}
