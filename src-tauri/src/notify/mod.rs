//! Notification System — Email alerts for copy events.
//!
//! Phase 1: Email via SMTP (lettre)
//! Future: SMS (Twilio), WeChat push (Server酱)
//!
//! Triggers:
//! - Copy completed
//! - Verification completed
//! - Error/anomaly detected

use serde::{Deserialize, Serialize};

/// Email notification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
    pub from_address: String,
    pub to_addresses: Vec<String>,
    pub use_tls: bool,
}

/// Notification event types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotifyEvent {
    CopyCompleted { job_id: String, file_count: usize, total_bytes: u64 },
    VerificationCompleted { job_id: String, all_passed: bool },
    ErrorDetected { job_id: String, message: String },
}
