//! Structured Error Code System for DIT System.
//!
//! Every runtime error has a unique code (E1xxx), category, and severity.
//! Error codes are organized by subsystem:
//!   E1000–E1099  COPY     File copy operations
//!   E1100–E1199  HASH     Hash computation & verification
//!   E1200–E1299  DB       Database operations
//!   E1300–E1399  CONFIG   Configuration management
//!   E1400–E1499  IO       Storage & filesystem I/O
//!   E1500–E1599  MHL      ASC MHL chain operations
//!   E1600–E1699  NETWORK  Email & network operations
//!   E1700–E1799  SYSTEM   Runtime & system errors

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─── Severity ────────────────────────────────────────────────────────────────

/// Error severity levels (descending order of impact).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// System cannot continue safely; data loss risk.
    Critical,
    /// Operation failed; user action needed.
    Error,
    /// Operation completed but with issues.
    Warning,
    /// Informational notice.
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Critical => write!(f, "critical"),
            Self::Error => write!(f, "error"),
            Self::Warning => write!(f, "warning"),
            Self::Info => write!(f, "info"),
        }
    }
}

// ─── Category ────────────────────────────────────────────────────────────────

/// Error category for grouping and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ErrorCategory {
    Copy,
    Hash,
    Db,
    Config,
    Io,
    Mhl,
    Network,
    System,
}

impl std::fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Copy => write!(f, "COPY"),
            Self::Hash => write!(f, "HASH"),
            Self::Db => write!(f, "DB"),
            Self::Config => write!(f, "CONFIG"),
            Self::Io => write!(f, "IO"),
            Self::Mhl => write!(f, "MHL"),
            Self::Network => write!(f, "NETWORK"),
            Self::System => write!(f, "SYSTEM"),
        }
    }
}

// ─── DitError ────────────────────────────────────────────────────────────────

/// Structured DIT error with unique error code.
#[derive(Debug, Error)]
pub enum DitError {
    // ── COPY (E1000–E1099) ───────────────────────────────
    #[error("[E1001] Failed to read source file: {path}")]
    CopyReadFailed {
        path: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1002] Failed to write destination file: {path}")]
    CopyWriteFailed {
        path: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1003] Insufficient disk space on {volume}: need {required}, have {available}")]
    CopyDiskFull {
        volume: String,
        required: u64,
        available: u64,
    },

    #[error("[E1004] Atomic rename failed for: {path}")]
    CopyRenameFailed {
        path: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1005] Source path not found: {path}")]
    CopySourceNotFound { path: String },

    #[error("[E1006] Destination path not writable: {path}")]
    CopyDestNotWritable { path: String },

    #[error("[E1007] Copy cancelled by user")]
    CopyCancelled,

    // ── HASH (E1100–E1199) ───────────────────────────────
    #[error("[E1101] Hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("[E1102] Hash computation failed for {path}")]
    HashComputeFailed {
        path: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1103] Unsupported hash algorithm: {algorithm}")]
    HashUnsupportedAlgorithm { algorithm: String },

    // ── DB (E1200–E1299) ─────────────────────────────────
    #[error("[E1201] Database connection failed")]
    DbConnectionFailed {
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1202] Database query failed: {query_desc}")]
    DbQueryFailed {
        query_desc: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1203] Database migration failed")]
    DbMigrationFailed {
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1204] Database lock timeout")]
    DbLockTimeout,

    #[error("[E1205] Database record not found: {desc}")]
    DbNotFound { desc: String },

    // ── CONFIG (E1300–E1399) ─────────────────────────────
    #[error("[E1301] Failed to parse config: {detail}")]
    ConfigParseFailed {
        detail: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1302] Failed to save config")]
    ConfigSaveFailed {
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1303] Invalid config value: {field} = {value}")]
    ConfigInvalidValue { field: String, value: String },

    // ── IO (E1400–E1499) ─────────────────────────────────
    #[error("[E1401] Device not found: {device}")]
    IoDeviceNotFound { device: String },

    #[error("[E1402] Permission denied: {path}")]
    IoPermissionDenied { path: String },

    #[error("[E1403] Path not accessible: {path}")]
    IoPathNotAccessible {
        path: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1404] Disk ejected during operation: {volume}")]
    IoDiskEjected { volume: String },

    #[error("[E1405] Temporary file cleanup failed: {path}")]
    IoTmpCleanupFailed { path: String },

    // ── MHL (E1500–E1599) ────────────────────────────────
    #[error("[E1501] MHL chain integrity error at {path}")]
    MhlChainCorrupt { path: String },

    #[error("[E1502] Failed to write MHL manifest: {path}")]
    MhlWriteFailed {
        path: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1503] MHL verification failed: {detail}")]
    MhlVerifyFailed { detail: String },

    // ── NETWORK (E1600–E1699) ────────────────────────────
    #[error("[E1601] SMTP connection failed: {host}")]
    SmtpConnectFailed {
        host: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("[E1602] SMTP authentication failed for {username}")]
    SmtpAuthFailed { username: String },

    #[error("[E1603] Email send failed")]
    EmailSendFailed {
        #[source]
        source: anyhow::Error,
    },

    // ── SYSTEM (E1700–E1799) ─────────────────────────────
    #[error("[E1701] Out of memory")]
    SystemOom,

    #[error("[E1702] Internal thread panic: {detail}")]
    SystemPanic { detail: String },

    #[error("[E1703] Lock poisoned: {resource}")]
    SystemLockPoisoned { resource: String },

    #[error("[E1704] Unknown error: {detail}")]
    SystemUnknown { detail: String },
}

// ─── Metadata accessors ──────────────────────────────────────────────────────

impl DitError {
    /// Return the unique error code string (e.g., "E1001").
    pub fn code(&self) -> &'static str {
        match self {
            Self::CopyReadFailed { .. } => "E1001",
            Self::CopyWriteFailed { .. } => "E1002",
            Self::CopyDiskFull { .. } => "E1003",
            Self::CopyRenameFailed { .. } => "E1004",
            Self::CopySourceNotFound { .. } => "E1005",
            Self::CopyDestNotWritable { .. } => "E1006",
            Self::CopyCancelled => "E1007",
            Self::HashMismatch { .. } => "E1101",
            Self::HashComputeFailed { .. } => "E1102",
            Self::HashUnsupportedAlgorithm { .. } => "E1103",
            Self::DbConnectionFailed { .. } => "E1201",
            Self::DbQueryFailed { .. } => "E1202",
            Self::DbMigrationFailed { .. } => "E1203",
            Self::DbLockTimeout => "E1204",
            Self::DbNotFound { .. } => "E1205",
            Self::ConfigParseFailed { .. } => "E1301",
            Self::ConfigSaveFailed { .. } => "E1302",
            Self::ConfigInvalidValue { .. } => "E1303",
            Self::IoDeviceNotFound { .. } => "E1401",
            Self::IoPermissionDenied { .. } => "E1402",
            Self::IoPathNotAccessible { .. } => "E1403",
            Self::IoDiskEjected { .. } => "E1404",
            Self::IoTmpCleanupFailed { .. } => "E1405",
            Self::MhlChainCorrupt { .. } => "E1501",
            Self::MhlWriteFailed { .. } => "E1502",
            Self::MhlVerifyFailed { .. } => "E1503",
            Self::SmtpConnectFailed { .. } => "E1601",
            Self::SmtpAuthFailed { .. } => "E1602",
            Self::EmailSendFailed { .. } => "E1603",
            Self::SystemOom => "E1701",
            Self::SystemPanic { .. } => "E1702",
            Self::SystemLockPoisoned { .. } => "E1703",
            Self::SystemUnknown { .. } => "E1704",
        }
    }

    /// Return the error category.
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::CopyReadFailed { .. }
            | Self::CopyWriteFailed { .. }
            | Self::CopyDiskFull { .. }
            | Self::CopyRenameFailed { .. }
            | Self::CopySourceNotFound { .. }
            | Self::CopyDestNotWritable { .. }
            | Self::CopyCancelled => ErrorCategory::Copy,

            Self::HashMismatch { .. }
            | Self::HashComputeFailed { .. }
            | Self::HashUnsupportedAlgorithm { .. } => ErrorCategory::Hash,

            Self::DbConnectionFailed { .. }
            | Self::DbQueryFailed { .. }
            | Self::DbMigrationFailed { .. }
            | Self::DbLockTimeout
            | Self::DbNotFound { .. } => ErrorCategory::Db,

            Self::ConfigParseFailed { .. }
            | Self::ConfigSaveFailed { .. }
            | Self::ConfigInvalidValue { .. } => ErrorCategory::Config,

            Self::IoDeviceNotFound { .. }
            | Self::IoPermissionDenied { .. }
            | Self::IoPathNotAccessible { .. }
            | Self::IoDiskEjected { .. }
            | Self::IoTmpCleanupFailed { .. } => ErrorCategory::Io,

            Self::MhlChainCorrupt { .. }
            | Self::MhlWriteFailed { .. }
            | Self::MhlVerifyFailed { .. } => ErrorCategory::Mhl,

            Self::SmtpConnectFailed { .. }
            | Self::SmtpAuthFailed { .. }
            | Self::EmailSendFailed { .. } => ErrorCategory::Network,

            Self::SystemOom
            | Self::SystemPanic { .. }
            | Self::SystemLockPoisoned { .. }
            | Self::SystemUnknown { .. } => ErrorCategory::System,
        }
    }

    /// Return the severity level.
    pub fn severity(&self) -> Severity {
        match self {
            // Critical — data loss risk
            Self::CopyDiskFull { .. }
            | Self::IoDiskEjected { .. }
            | Self::HashMismatch { .. }
            | Self::SystemOom
            | Self::DbConnectionFailed { .. }
            | Self::SystemPanic { .. } => Severity::Critical,

            // Warning — operation completed with issues
            Self::ConfigInvalidValue { .. }
            | Self::CopyCancelled
            | Self::IoTmpCleanupFailed { .. } => Severity::Warning,

            // Error — everything else
            _ => Severity::Error,
        }
    }
}

/// Convert from anyhow::Error — attempts downcast, falls back to SystemUnknown.
impl From<anyhow::Error> for DitError {
    fn from(err: anyhow::Error) -> Self {
        match err.downcast::<DitError>() {
            Ok(dit_err) => dit_err,
            Err(other) => DitError::SystemUnknown {
                detail: other.to_string(),
            },
        }
    }
}

// ─── Serializable error info for frontend ────────────────────────────────────

/// Structured error info that can be sent to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorInfo {
    /// Unique error code, e.g. "E1001"
    pub code: String,
    /// Category name, e.g. "COPY"
    pub category: String,
    /// Severity level, e.g. "critical"
    pub severity: String,
    /// Human-readable error message
    pub message: String,
    /// Extra structured context (optional)
    pub context: Option<serde_json::Value>,
}

impl ErrorInfo {
    /// Create from a DitError with optional extra context.
    pub fn from_dit_error(err: &DitError, context: Option<serde_json::Value>) -> Self {
        Self {
            code: err.code().to_string(),
            category: err.category().to_string(),
            severity: err.severity().to_string(),
            message: err.to_string(),
            context,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        let err = DitError::CopyReadFailed {
            path: "/mnt/card/clip.mov".into(),
            source: anyhow::anyhow!("I/O error"),
        };
        assert_eq!(err.code(), "E1001");
        assert_eq!(err.category(), ErrorCategory::Copy);
        assert_eq!(err.severity(), Severity::Error);
        assert!(err.to_string().contains("E1001"));
        assert!(err.to_string().contains("clip.mov"));
    }

    #[test]
    fn test_critical_severity() {
        let err = DitError::CopyDiskFull {
            volume: "/Volumes/DEST".into(),
            required: 100_000_000,
            available: 50_000_000,
        };
        assert_eq!(err.code(), "E1003");
        assert_eq!(err.severity(), Severity::Critical);
    }

    #[test]
    fn test_hash_mismatch() {
        let err = DitError::HashMismatch {
            path: "clip001.mov".into(),
            expected: "aabb".into(),
            actual: "ccdd".into(),
        };
        assert_eq!(err.code(), "E1101");
        assert_eq!(err.category(), ErrorCategory::Hash);
        assert_eq!(err.severity(), Severity::Critical);
    }

    #[test]
    fn test_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("something went wrong");
        let dit_err = DitError::from(anyhow_err);
        assert_eq!(dit_err.code(), "E1704");
        assert_eq!(dit_err.category(), ErrorCategory::System);
    }

    #[test]
    fn test_error_info_serialization() {
        let err = DitError::DbLockTimeout;
        let info = ErrorInfo::from_dit_error(&err, None);
        assert_eq!(info.code, "E1204");
        assert_eq!(info.category, "DB");
        assert_eq!(info.severity, "error");
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("E1204"));
    }

    #[test]
    fn test_severity_display() {
        assert_eq!(Severity::Critical.to_string(), "critical");
        assert_eq!(Severity::Warning.to_string(), "warning");
    }

    #[test]
    fn test_all_codes_unique() {
        // Verify all error codes are unique by collecting them
        let errors: Vec<Box<dyn Fn() -> &'static str>> = vec![
            Box::new(|| DitError::CopyReadFailed { path: String::new(), source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::CopyWriteFailed { path: String::new(), source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::CopyDiskFull { volume: String::new(), required: 0, available: 0 }.code()),
            Box::new(|| DitError::CopyRenameFailed { path: String::new(), source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::CopySourceNotFound { path: String::new() }.code()),
            Box::new(|| DitError::CopyDestNotWritable { path: String::new() }.code()),
            Box::new(|| DitError::CopyCancelled.code()),
            Box::new(|| DitError::HashMismatch { path: String::new(), expected: String::new(), actual: String::new() }.code()),
            Box::new(|| DitError::HashComputeFailed { path: String::new(), source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::HashUnsupportedAlgorithm { algorithm: String::new() }.code()),
            Box::new(|| DitError::DbConnectionFailed { source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::DbQueryFailed { query_desc: String::new(), source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::DbMigrationFailed { source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::DbLockTimeout.code()),
            Box::new(|| DitError::DbNotFound { desc: String::new() }.code()),
            Box::new(|| DitError::ConfigParseFailed { detail: String::new(), source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::ConfigSaveFailed { source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::ConfigInvalidValue { field: String::new(), value: String::new() }.code()),
            Box::new(|| DitError::IoDeviceNotFound { device: String::new() }.code()),
            Box::new(|| DitError::IoPermissionDenied { path: String::new() }.code()),
            Box::new(|| DitError::IoPathNotAccessible { path: String::new(), source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::IoDiskEjected { volume: String::new() }.code()),
            Box::new(|| DitError::IoTmpCleanupFailed { path: String::new() }.code()),
            Box::new(|| DitError::MhlChainCorrupt { path: String::new() }.code()),
            Box::new(|| DitError::MhlWriteFailed { path: String::new(), source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::MhlVerifyFailed { detail: String::new() }.code()),
            Box::new(|| DitError::SmtpConnectFailed { host: String::new(), source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::SmtpAuthFailed { username: String::new() }.code()),
            Box::new(|| DitError::EmailSendFailed { source: anyhow::anyhow!("") }.code()),
            Box::new(|| DitError::SystemOom.code()),
            Box::new(|| DitError::SystemPanic { detail: String::new() }.code()),
            Box::new(|| DitError::SystemLockPoisoned { resource: String::new() }.code()),
            Box::new(|| DitError::SystemUnknown { detail: String::new() }.code()),
        ];
        let codes: Vec<&str> = errors.iter().map(|f| f()).collect();
        let mut unique = codes.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(codes.len(), unique.len(), "Duplicate error codes detected");
    }
}
