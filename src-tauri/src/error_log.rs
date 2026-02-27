//! Error Log — persistent error tracking in SQLite.
//!
//! Records every DitError that occurs at runtime with full context,
//! enabling post-mortem analysis and bug reporting.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::DitError;

// ─── Data types ──────────────────────────────────────────────────────────────

/// A single error log entry as stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorLogEntry {
    pub id: i64,
    pub timestamp: String,
    pub error_code: String,
    pub severity: String,
    pub category: String,
    pub module: String,
    pub message: String,
    pub context_json: Option<String>,
    pub job_id: Option<String>,
    pub resolved: bool,
    pub resolved_at: Option<String>,
    pub app_version: Option<String>,
}

/// Summary counts by severity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorLogSummary {
    pub total: usize,
    pub critical: usize,
    pub error: usize,
    pub warning: usize,
    pub info: usize,
    pub unresolved: usize,
}

/// Filter for querying the error log.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorLogFilter {
    pub severity: Option<String>,
    pub category: Option<String>,
    pub job_id: Option<String>,
    pub resolved: Option<bool>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

// ─── Write operations ────────────────────────────────────────────────────────

/// Insert an error log entry from a DitError.
/// Returns the new row ID.
pub fn log_error(
    conn: &Connection,
    dit_err: &DitError,
    module: &str,
    job_id: Option<&str>,
    context: Option<&serde_json::Value>,
) -> Result<i64> {
    let ctx_str = context.map(|c| serde_json::to_string(c).unwrap_or_default());

    conn.execute(
        "INSERT INTO error_log (error_code, severity, category, module, message, context_json, job_id, app_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            dit_err.code(),
            dit_err.severity().to_string(),
            dit_err.category().to_string(),
            module,
            dit_err.to_string(),
            ctx_str,
            job_id,
            env!("CARGO_PKG_VERSION"),
        ],
    )
    .context("Failed to insert error log entry")?;

    Ok(conn.last_insert_rowid())
}

/// Log an error from raw components (useful when DitError is not available).
pub fn log_raw_error(
    conn: &Connection,
    code: &str,
    severity: &str,
    category: &str,
    module: &str,
    message: &str,
    job_id: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO error_log (error_code, severity, category, module, message, job_id, app_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            code,
            severity,
            category,
            module,
            message,
            job_id,
            env!("CARGO_PKG_VERSION"),
        ],
    )
    .context("Failed to insert raw error log entry")?;

    Ok(conn.last_insert_rowid())
}

// ─── Read operations ─────────────────────────────────────────────────────────

/// Query error log with optional filters.
/// Results ordered by timestamp DESC (most recent first).
pub fn query_error_log(conn: &Connection, filter: &ErrorLogFilter) -> Result<Vec<ErrorLogEntry>> {
    let mut sql = String::from(
        "SELECT id, timestamp, error_code, severity, category, module, message,
                context_json, job_id, resolved, resolved_at, app_version
         FROM error_log WHERE 1=1",
    );
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref sev) = filter.severity {
        param_values.push(Box::new(sev.clone()));
        sql.push_str(&format!(" AND severity = ?{}", param_values.len()));
    }
    if let Some(ref cat) = filter.category {
        param_values.push(Box::new(cat.clone()));
        sql.push_str(&format!(" AND category = ?{}", param_values.len()));
    }
    if let Some(ref jid) = filter.job_id {
        param_values.push(Box::new(jid.clone()));
        sql.push_str(&format!(" AND job_id = ?{}", param_values.len()));
    }
    if let Some(resolved) = filter.resolved {
        param_values.push(Box::new(resolved as i32));
        sql.push_str(&format!(" AND resolved = ?{}", param_values.len()));
    }

    sql.push_str(" ORDER BY timestamp DESC");

    let limit = filter.limit.unwrap_or(200).min(1000);
    let offset = filter.offset.unwrap_or(0);
    sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn
        .prepare(&sql)
        .context("Failed to prepare error_log query")?;

    let entries = stmt
        .query_map(params_ref.as_slice(), |row| {
            Ok(ErrorLogEntry {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                error_code: row.get(2)?,
                severity: row.get(3)?,
                category: row.get(4)?,
                module: row.get(5)?,
                message: row.get(6)?,
                context_json: row.get(7)?,
                job_id: row.get(8)?,
                resolved: row.get::<_, i32>(9)? != 0,
                resolved_at: row.get(10)?,
                app_version: row.get(11)?,
            })
        })
        .context("Failed to query error_log")?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to read error_log rows")?;

    Ok(entries)
}

/// Get error log summary (counts by severity).
pub fn error_log_summary(conn: &Connection) -> Result<ErrorLogSummary> {
    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM error_log", [], |row| row.get(0))
        .unwrap_or(0);

    let critical: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM error_log WHERE severity = 'critical'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let error: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM error_log WHERE severity = 'error'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let warning: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM error_log WHERE severity = 'warning'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let info: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM error_log WHERE severity = 'info'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let unresolved: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM error_log WHERE resolved = 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    Ok(ErrorLogSummary {
        total,
        critical,
        error,
        warning,
        info,
        unresolved,
    })
}

// ─── Update / Delete ─────────────────────────────────────────────────────────

/// Mark an error as resolved.
pub fn resolve_error(conn: &Connection, id: i64) -> Result<bool> {
    let rows = conn
        .execute(
            "UPDATE error_log SET resolved = 1, resolved_at = datetime('now') WHERE id = ?1",
            params![id],
        )
        .context("Failed to resolve error log entry")?;
    Ok(rows > 0)
}

/// Clear error log entries older than the given number of days.
/// If `older_than_days` is None, clears all entries.
/// Returns the number of deleted rows.
pub fn clear_error_log(conn: &Connection, older_than_days: Option<u32>) -> Result<usize> {
    let rows = if let Some(days) = older_than_days {
        conn.execute(
            "DELETE FROM error_log WHERE timestamp < datetime('now', ?1)",
            params![format!("-{} days", days)],
        )
        .context("Failed to clear old error log entries")?
    } else {
        conn.execute("DELETE FROM error_log", [])
            .context("Failed to clear all error log entries")?
    };
    Ok(rows)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn setup_db() -> Connection {
        let conn = db::init_database(":memory:").unwrap();
        conn
    }

    #[test]
    fn test_log_and_query_error() {
        let conn = setup_db();
        let err = DitError::CopySourceNotFound {
            path: "/mnt/card/missing.mov".into(),
        };

        let id = log_error(&conn, &err, "workflow", Some("job-001"), None).unwrap();
        assert!(id > 0);

        let entries = query_error_log(&conn, &ErrorLogFilter::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].error_code, "E1005");
        assert_eq!(entries[0].severity, "error");
        assert_eq!(entries[0].category, "COPY");
        assert_eq!(entries[0].module, "workflow");
        assert_eq!(entries[0].job_id, Some("job-001".to_string()));
        assert!(!entries[0].resolved);
    }

    #[test]
    fn test_filter_by_severity() {
        let conn = setup_db();

        log_error(
            &conn,
            &DitError::CopyDiskFull {
                volume: "/dest".into(),
                required: 100,
                available: 50,
            },
            "copy_engine",
            None,
            None,
        )
        .unwrap();

        log_error(
            &conn,
            &DitError::CopySourceNotFound {
                path: "/src".into(),
            },
            "workflow",
            None,
            None,
        )
        .unwrap();

        let critical_only = query_error_log(
            &conn,
            &ErrorLogFilter {
                severity: Some("critical".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(critical_only.len(), 1);
        assert_eq!(critical_only[0].error_code, "E1003");
    }

    #[test]
    fn test_resolve_error() {
        let conn = setup_db();
        let err = DitError::DbLockTimeout;
        let id = log_error(&conn, &err, "commands", None, None).unwrap();

        assert!(resolve_error(&conn, id).unwrap());

        let entries = query_error_log(
            &conn,
            &ErrorLogFilter {
                resolved: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].resolved);
        assert!(entries[0].resolved_at.is_some());
    }

    #[test]
    fn test_clear_error_log() {
        let conn = setup_db();
        for _ in 0..5 {
            log_error(
                &conn,
                &DitError::SystemUnknown {
                    detail: "test".into(),
                },
                "test",
                None,
                None,
            )
            .unwrap();
        }

        let deleted = clear_error_log(&conn, None).unwrap();
        assert_eq!(deleted, 5);

        let summary = error_log_summary(&conn).unwrap();
        assert_eq!(summary.total, 0);
    }

    #[test]
    fn test_error_log_summary() {
        let conn = setup_db();

        log_error(
            &conn,
            &DitError::CopyDiskFull {
                volume: "a".into(),
                required: 1,
                available: 0,
            },
            "test",
            None,
            None,
        )
        .unwrap();
        log_error(
            &conn,
            &DitError::CopyDiskFull {
                volume: "b".into(),
                required: 1,
                available: 0,
            },
            "test",
            None,
            None,
        )
        .unwrap();
        log_error(
            &conn,
            &DitError::CopySourceNotFound { path: "c".into() },
            "test",
            None,
            None,
        )
        .unwrap();
        log_error(
            &conn,
            &DitError::ConfigInvalidValue {
                field: "x".into(),
                value: "y".into(),
            },
            "test",
            None,
            None,
        )
        .unwrap();

        let summary = error_log_summary(&conn).unwrap();
        assert_eq!(summary.total, 4);
        assert_eq!(summary.critical, 2);
        assert_eq!(summary.error, 1);
        assert_eq!(summary.warning, 1);
        assert_eq!(summary.unresolved, 4);
    }

    #[test]
    fn test_log_raw_error() {
        let conn = setup_db();
        let id = log_raw_error(
            &conn,
            "E9999",
            "warning",
            "SYSTEM",
            "test_module",
            "test raw error",
            None,
        )
        .unwrap();
        assert!(id > 0);

        let entries = query_error_log(&conn, &ErrorLogFilter::default()).unwrap();
        assert_eq!(entries[0].error_code, "E9999");
    }
}
