//! Report Generation — Professional PDF and HTML reports.
//!
//! Phase 1 report types:
//! - Shooting Day Report: daily summary of files, sizes, durations, backup status
//! - Volume Report: per-volume summary with hash verification and MHL chain info
//!
//! Output formats: PDF and HTML
//! Supports custom templates and embedded production company logos.

use serde::{Deserialize, Serialize};

/// Type of report to generate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReportType {
    ShootingDay,
    Volume,
}

/// Output format for reports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReportFormat {
    PDF,
    HTML,
}

/// Configuration for report generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportConfig {
    pub report_type: ReportType,
    pub format: ReportFormat,
    pub output_path: String,
    pub project_name: Option<String>,
    pub production_company: Option<String>,
    pub logo_path: Option<String>,
}
