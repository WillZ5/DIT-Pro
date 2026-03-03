//! Checkpoint / Recovery System — Crash-safe task state persistence.
//!
//! Uses SQLite WAL mode to maintain task state across crashes.
//! State machine: pending → copying → verifying → completed | failed
//!
//! Recovery flow:
//! 1. Scan copy_tasks where status != 'completed'
//! 2. Clean up orphaned .tmp files for interrupted tasks
//! 3. Reset 'copying'/'verifying' tasks back to 'pending'
//! 4. Resume from the last completed file

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::copy_engine::atomic_writer;

/// Task status values stored in the database
pub const STATUS_PENDING: &str = "pending";
pub const STATUS_COPYING: &str = "copying";
pub const STATUS_VERIFYING: &str = "verifying";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_SKIPPED: &str = "skipped";
pub const STATUS_PAUSED: &str = "paused";
pub const STATUS_TERMINATED: &str = "terminated";
pub const STATUS_CONFLICT: &str = "conflict";

/// All hash values for a completed copy task
#[derive(Debug, Clone, Default)]
pub struct TaskHashes {
    pub xxh64: Option<String>,
    pub sha256: Option<String>,
    pub md5: Option<String>,
    pub xxh128: Option<String>,
    pub xxh3: Option<String>,
}

/// A checkpoint-managed copy task record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub task_id: String,
    pub job_id: String,
    pub source_path: String,
    pub dest_path: String,
    pub file_size: i64,
    pub status: String,
    pub hash_xxh64: Option<String>,
    pub hash_sha256: Option<String>,
    pub error_msg: Option<String>,
    pub retry_count: i32,
}

/// Create a new job in the database
pub fn create_job(
    conn: &Connection,
    job_id: &str,
    name: &str,
    source_path: &str,
    config_json: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO jobs (id, name, status, source_path, config_json) VALUES (?1, ?2, 'pending', ?3, ?4)",
        params![job_id, name, source_path, config_json],
    )?;
    Ok(())
}

/// Get stored config JSON for a job (used for re-run)
pub fn get_job_config(conn: &Connection, job_id: &str) -> Result<Option<String>> {
    let config: Option<String> = conn.query_row(
        "SELECT config_json FROM jobs WHERE id = ?1",
        params![job_id],
        |row| row.get(0),
    )?;
    Ok(config)
}

/// Insert a new copy task
pub fn insert_task(
    conn: &Connection,
    task_id: &str,
    job_id: &str,
    source_path: &str,
    dest_path: &str,
    file_size: u64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO copy_tasks (id, job_id, source_path, dest_path, file_size, status)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
        params![task_id, job_id, source_path, dest_path, file_size as i64],
    )?;
    Ok(())
}

/// Update task status
pub fn update_task_status(conn: &Connection, task_id: &str, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE copy_tasks SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![status, task_id],
    )?;
    Ok(())
}

/// Update task status with hash results (all algorithms)
pub fn update_task_completed(conn: &Connection, task_id: &str, hashes: &TaskHashes) -> Result<()> {
    conn.execute(
        "UPDATE copy_tasks SET status = 'completed',
         hash_xxh64 = ?1, hash_sha256 = ?2, hash_md5 = ?3, hash_xxh128 = ?4, hash_xxh3 = ?5,
         updated_at = datetime('now') WHERE id = ?6",
        params![
            hashes.xxh64,
            hashes.sha256,
            hashes.md5,
            hashes.xxh128,
            hashes.xxh3,
            task_id
        ],
    )?;
    Ok(())
}

/// Update task as failed with error message.
/// Also appends the error to `retry_note` so the report can show the failure history
/// even if the file is later retried and succeeds.
pub fn update_task_failed(conn: &Connection, task_id: &str, error_msg: &str) -> Result<()> {
    // Append to retry_note: "Round 1 verify failed: <reason>"
    let retry_count: i32 = conn
        .query_row(
            "SELECT retry_count FROM copy_tasks WHERE id = ?1",
            params![task_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let round = retry_count + 1;
    let note_entry = format!("Round {} failed: {}", round, error_msg);

    conn.execute(
        "UPDATE copy_tasks SET status = 'failed', error_msg = ?1,
         retry_count = retry_count + 1,
         retry_note = CASE
             WHEN retry_note IS NULL THEN ?3
             ELSE retry_note || '; ' || ?3
         END,
         updated_at = datetime('now') WHERE id = ?2",
        params![error_msg, task_id, note_entry],
    )?;
    Ok(())
}

/// Append a success note to retry_note after a successful retry.
/// Called when a previously-failed task is re-copied and re-verified successfully.
pub fn append_retry_success(conn: &Connection, task_id: &str) -> Result<()> {
    let retry_count: i32 = conn
        .query_row(
            "SELECT retry_count FROM copy_tasks WHERE id = ?1",
            params![task_id],
            |row| row.get(0),
        )
        .unwrap_or(1);
    let note_entry = format!("Round {} retry succeeded", retry_count + 1);

    conn.execute(
        "UPDATE copy_tasks SET
         retry_note = CASE
             WHEN retry_note IS NULL THEN ?2
             ELSE retry_note || '; ' || ?2
         END,
         updated_at = datetime('now') WHERE id = ?1",
        params![task_id, note_entry],
    )?;
    Ok(())
}

/// Update job-level status directly
pub fn update_job_status(conn: &Connection, job_id: &str, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE jobs SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![status, job_id],
    )?;
    Ok(())
}

/// Mark task as skipped (existing file verified identical)
pub fn update_task_skipped(conn: &Connection, task_id: &str, hashes: &TaskHashes) -> Result<()> {
    conn.execute(
        "UPDATE copy_tasks SET status = 'skipped',
         hash_xxh64 = ?1, hash_sha256 = ?2, hash_md5 = ?3, hash_xxh128 = ?4, hash_xxh3 = ?5,
         updated_at = datetime('now') WHERE id = ?6",
        params![
            hashes.xxh64,
            hashes.sha256,
            hashes.md5,
            hashes.xxh128,
            hashes.xxh3,
            task_id
        ],
    )?;
    Ok(())
}

/// Get all pending tasks for a job
pub fn get_pending_tasks(conn: &Connection, job_id: &str) -> Result<Vec<CheckpointRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, job_id, source_path, dest_path, file_size, status,
                hash_xxh64, hash_sha256, error_msg, retry_count
         FROM copy_tasks WHERE job_id = ?1 AND status = 'pending'
         ORDER BY rowid ASC",
    )?;

    let records = stmt
        .query_map(params![job_id], |row| {
            Ok(CheckpointRecord {
                task_id: row.get(0)?,
                job_id: row.get(1)?,
                source_path: row.get(2)?,
                dest_path: row.get(3)?,
                file_size: row.get(4)?,
                status: row.get(5)?,
                hash_xxh64: row.get(6)?,
                hash_sha256: row.get(7)?,
                error_msg: row.get(8)?,
                retry_count: row.get(9)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to read checkpoint records")?;

    Ok(records)
}

/// Get tasks that were interrupted or failed (recoverable tasks)
pub fn get_interrupted_tasks(conn: &Connection, job_id: &str) -> Result<Vec<CheckpointRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, job_id, source_path, dest_path, file_size, status,
                hash_xxh64, hash_sha256, error_msg, retry_count
         FROM copy_tasks WHERE job_id = ?1 AND status IN ('copying', 'verifying', 'failed')
         ORDER BY rowid ASC",
    )?;

    let records = stmt
        .query_map(params![job_id], |row| {
            Ok(CheckpointRecord {
                task_id: row.get(0)?,
                job_id: row.get(1)?,
                source_path: row.get(2)?,
                dest_path: row.get(3)?,
                file_size: row.get(4)?,
                status: row.get(5)?,
                hash_xxh64: row.get(6)?,
                hash_sha256: row.get(7)?,
                error_msg: row.get(8)?,
                retry_count: row.get(9)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to read interrupted tasks")?;

    Ok(records)
}

/// Recovery procedure: clean up .tmp files and reset interrupted tasks.
///
/// Returns the list of tasks that were reset and are now ready to retry.
pub async fn recover_job(conn: &Connection, job_id: &str) -> Result<Vec<CheckpointRecord>> {
    // 1. Find interrupted tasks
    let interrupted = get_interrupted_tasks(conn, job_id)?;

    for task in &interrupted {
        // 2. Clean up .tmp files
        let dest = Path::new(&task.dest_path);
        let tmp_path = atomic_writer::AtomicWriter::temp_path_for(dest);
        if tmp_path.exists() {
            tokio::fs::remove_file(&tmp_path).await.ok();
            log::info!("Cleaned up orphaned tmp file: {:?}", tmp_path);
        }

        // 3. Reset to pending
        update_task_status(conn, &task.task_id, STATUS_PENDING)?;
        log::info!(
            "Reset interrupted task {} ({} -> {})",
            task.task_id,
            task.source_path,
            task.dest_path
        );
    }

    // 4. Return all pending tasks (original pending + reset ones)
    get_pending_tasks(conn, job_id)
}

/// Get job progress summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobProgress {
    pub job_id: String,
    pub total_tasks: usize,
    pub completed: usize,
    pub pending: usize,
    pub copying: usize,
    pub failed: usize,
    pub skipped: usize,
    pub total_bytes: u64,
    pub completed_bytes: u64,
}

pub fn get_job_progress(conn: &Connection, job_id: &str) -> Result<JobProgress> {
    let total_tasks: usize = conn.query_row(
        "SELECT COUNT(*) FROM copy_tasks WHERE job_id = ?1",
        params![job_id],
        |row| row.get(0),
    )?;

    let completed: usize = conn.query_row(
        "SELECT COUNT(*) FROM copy_tasks WHERE job_id = ?1 AND status IN ('completed', 'skipped')",
        params![job_id],
        |row| row.get(0),
    )?;

    let pending: usize = conn.query_row(
        "SELECT COUNT(*) FROM copy_tasks WHERE job_id = ?1 AND status = 'pending'",
        params![job_id],
        |row| row.get(0),
    )?;

    let copying: usize = conn.query_row(
        "SELECT COUNT(*) FROM copy_tasks WHERE job_id = ?1 AND status IN ('copying', 'verifying')",
        params![job_id],
        |row| row.get(0),
    )?;

    let failed: usize = conn.query_row(
        "SELECT COUNT(*) FROM copy_tasks WHERE job_id = ?1 AND status = 'failed'",
        params![job_id],
        |row| row.get(0),
    )?;

    let skipped: usize = conn.query_row(
        "SELECT COUNT(*) FROM copy_tasks WHERE job_id = ?1 AND status = 'skipped'",
        params![job_id],
        |row| row.get(0),
    )?;

    let total_bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(file_size), 0) FROM copy_tasks WHERE job_id = ?1",
        params![job_id],
        |row| row.get(0),
    )?;

    let completed_bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(file_size), 0) FROM copy_tasks WHERE job_id = ?1 AND status = 'completed'",
        params![job_id],
        |row| row.get(0),
    )?;

    let skipped_bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(file_size), 0) FROM copy_tasks WHERE job_id = ?1 AND status = 'skipped'",
        params![job_id],
        |row| row.get(0),
    )?;

    Ok(JobProgress {
        job_id: job_id.to_string(),
        total_tasks,
        completed,
        pending,
        copying,
        failed,
        skipped,
        total_bytes: total_bytes as u64,
        completed_bytes: (completed_bytes + skipped_bytes) as u64,
    })
}

/// Delete a single job and all its tasks by job ID
pub fn delete_job_by_id(conn: &Connection, job_id: &str) -> Result<()> {
    conn.execute("DELETE FROM copy_tasks WHERE job_id = ?1", params![job_id])?;
    let deleted = conn.execute("DELETE FROM jobs WHERE id = ?1", params![job_id])?;
    if deleted == 0 {
        anyhow::bail!("Job not found: {}", job_id);
    }
    Ok(())
}

/// Clear job records older than the given number of days
pub fn clear_old_jobs(conn: &Connection, days: u32) -> Result<usize> {
    let _deleted_tasks: usize = conn.execute(
        "DELETE FROM copy_tasks WHERE job_id IN (
            SELECT id FROM jobs WHERE created_at < datetime('now', ?1)
        )",
        params![format!("-{} days", days)],
    )?;
    let deleted_jobs: usize = conn.execute(
        "DELETE FROM jobs WHERE created_at < datetime('now', ?1)",
        params![format!("-{} days", days)],
    )?;
    Ok(deleted_jobs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        // Reuse the schema from db module
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY, name TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                source_path TEXT NOT NULL,
                config_json TEXT,
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

    #[test]
    fn test_create_job_and_tasks() {
        let conn = setup_test_db();
        create_job(&conn, "job-1", "Day 1 Offload", "/Volumes/CARD_A", None).unwrap();
        insert_task(
            &conn,
            "t-1",
            "job-1",
            "/src/clip1.mov",
            "/dst/clip1.mov",
            1000,
        )
        .unwrap();
        insert_task(
            &conn,
            "t-2",
            "job-1",
            "/src/clip2.mov",
            "/dst/clip2.mov",
            2000,
        )
        .unwrap();

        let pending = get_pending_tasks(&conn, "job-1").unwrap();
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn test_task_status_transitions() {
        let conn = setup_test_db();
        create_job(&conn, "job-1", "Test", "/src", None).unwrap();
        insert_task(&conn, "t-1", "job-1", "/src/a.mov", "/dst/a.mov", 500).unwrap();

        update_task_status(&conn, "t-1", STATUS_COPYING).unwrap();
        let pending = get_pending_tasks(&conn, "job-1").unwrap();
        assert_eq!(pending.len(), 0); // no longer pending

        update_task_completed(
            &conn,
            "t-1",
            &TaskHashes {
                xxh64: Some("abc123".into()),
                sha256: Some("def456".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let progress = get_job_progress(&conn, "job-1").unwrap();
        assert_eq!(progress.completed, 1);
        assert_eq!(progress.completed_bytes, 500);
    }

    #[test]
    fn test_failure_and_retry_count() {
        let conn = setup_test_db();
        create_job(&conn, "job-1", "Test", "/src", None).unwrap();
        insert_task(&conn, "t-1", "job-1", "/src/a.mov", "/dst/a.mov", 500).unwrap();

        update_task_failed(&conn, "t-1", "IO error: disk full").unwrap();
        update_task_failed(&conn, "t-1", "IO error: disk full").unwrap();

        let progress = get_job_progress(&conn, "job-1").unwrap();
        assert_eq!(progress.failed, 1);
    }

    #[test]
    fn test_delete_job_by_id() {
        let conn = setup_test_db();
        create_job(&conn, "job-1", "To Delete", "/src", None).unwrap();
        insert_task(&conn, "t-1", "job-1", "/src/a.mov", "/dst/a.mov", 100).unwrap();
        insert_task(&conn, "t-2", "job-1", "/src/b.mov", "/dst/b.mov", 200).unwrap();

        // Delete should remove both tasks and the job
        delete_job_by_id(&conn, "job-1").unwrap();

        let progress = get_job_progress(&conn, "job-1");
        assert!(progress.is_ok());
        assert_eq!(progress.unwrap().total_tasks, 0);

        // Deleting again should fail
        assert!(delete_job_by_id(&conn, "job-1").is_err());
    }

    #[tokio::test]
    async fn test_recover_interrupted_tasks() {
        let conn = setup_test_db();
        create_job(&conn, "job-1", "Test", "/src", None).unwrap();
        insert_task(&conn, "t-1", "job-1", "/src/a.mov", "/dst/a.mov", 100).unwrap();
        insert_task(&conn, "t-2", "job-1", "/src/b.mov", "/dst/b.mov", 200).unwrap();
        insert_task(&conn, "t-3", "job-1", "/src/c.mov", "/dst/c.mov", 300).unwrap();

        // Simulate: t-1 completed, t-2 was copying when crash happened
        update_task_completed(
            &conn,
            "t-1",
            &TaskHashes {
                xxh64: Some("hash1".into()),
                ..Default::default()
            },
        )
        .unwrap();
        update_task_status(&conn, "t-2", STATUS_COPYING).unwrap();

        let recovered = recover_job(&conn, "job-1").await.unwrap();
        // t-2 should be reset to pending, t-3 was already pending
        assert_eq!(recovered.len(), 2);

        let progress = get_job_progress(&conn, "job-1").unwrap();
        assert_eq!(progress.completed, 1);
        assert_eq!(progress.pending, 2);
    }
}
