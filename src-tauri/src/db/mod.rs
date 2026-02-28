//! Database Layer — SQLite with WAL mode for crash-safe persistence.
//!
//! Stores:
//! - Copy task state (checkpoint/recovery)
//! - File metadata
//! - Volume tracking info
//! - Job history
//! - Hash records
//! - Error log (structured error tracking)

use anyhow::Result;
use rusqlite::Connection;

/// Initialize the SQLite database with WAL mode and create tables
pub fn init_database(db_path: &str) -> Result<Connection> {
    let conn = Connection::open(db_path)?;

    // Enable WAL mode for crash safety
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    // Set busy timeout to avoid hangs on WAL contention (5 seconds)
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    // Create core tables
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS jobs (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            status      TEXT NOT NULL DEFAULT 'pending',
            source_path TEXT NOT NULL,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS copy_tasks (
            id          TEXT PRIMARY KEY,
            job_id      TEXT NOT NULL REFERENCES jobs(id),
            source_path TEXT NOT NULL,
            dest_path   TEXT NOT NULL,
            file_size   INTEGER NOT NULL DEFAULT 0,
            status      TEXT NOT NULL DEFAULT 'pending',
            hash_xxh64  TEXT,
            hash_sha256 TEXT,
            hash_md5    TEXT,
            hash_xxh128 TEXT,
            hash_xxh3   TEXT,
            error_msg   TEXT,
            retry_count INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS volumes (
            id              TEXT PRIMARY KEY,
            name            TEXT NOT NULL,
            mount_point     TEXT,
            total_bytes     INTEGER,
            available_bytes INTEGER,
            device_type     TEXT,
            serial_number   TEXT,
            last_seen_at    TEXT,
            last_seen_by    TEXT
        );

        CREATE TABLE IF NOT EXISTS hash_records (
            id          TEXT PRIMARY KEY,
            task_id     TEXT NOT NULL REFERENCES copy_tasks(id),
            algorithm   TEXT NOT NULL,
            hex_digest  TEXT NOT NULL,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS error_log (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp    TEXT NOT NULL DEFAULT (datetime('now')),
            error_code   TEXT NOT NULL,
            severity     TEXT NOT NULL,
            category     TEXT NOT NULL,
            module       TEXT NOT NULL,
            message      TEXT NOT NULL,
            context_json TEXT,
            job_id       TEXT,
            resolved     INTEGER NOT NULL DEFAULT 0,
            resolved_at  TEXT,
            app_version  TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_copy_tasks_job_id ON copy_tasks(job_id);
        CREATE INDEX IF NOT EXISTS idx_copy_tasks_status ON copy_tasks(status);
        CREATE INDEX IF NOT EXISTS idx_hash_records_task_id ON hash_records(task_id);
        CREATE INDEX IF NOT EXISTS idx_error_log_timestamp ON error_log(timestamp);
        CREATE INDEX IF NOT EXISTS idx_error_log_severity ON error_log(severity);
        CREATE INDEX IF NOT EXISTS idx_error_log_job_id ON error_log(job_id);
        CREATE INDEX IF NOT EXISTS idx_error_log_code ON error_log(error_code);
        ",
    )?;

    // Migration: add hash columns for MD5, XXH128, XXH3 (safe to re-run)
    for col in ["hash_md5", "hash_xxh128", "hash_xxh3"] {
        let sql = format!("ALTER TABLE copy_tasks ADD COLUMN {} TEXT", col);
        // Ignore error if column already exists
        let _ = conn.execute_batch(&sql);
    }

    Ok(conn)
}
