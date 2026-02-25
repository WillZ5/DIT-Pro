//! ASC MHL (Media Hash List) — Chain of custody tracking.
//!
//! Implements the ASC MHL standard for:
//! - History creation and continuation
//! - Volume sealing
//! - Flattened manifests for remote verification
//! - Ignore patterns for system/temp files
//! - Creator info embedding (operator, software version, OS, device serial)

use serde::{Deserialize, Serialize};

/// Creator information embedded in MHL records
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlCreatorInfo {
    pub software_name: String,
    pub software_version: String,
    pub os_name: String,
    pub os_version: String,
    pub operator_name: Option<String>,
    pub operator_contact: Option<String>,
    pub device_serial: Option<String>,
}

/// Configuration for MHL generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlConfig {
    pub creator_info: MhlCreatorInfo,
    /// File patterns to ignore (e.g., ".DS_Store", "*.tmp")
    pub ignore_patterns: Vec<String>,
}
