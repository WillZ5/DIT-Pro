//! Copy Engine — Multi-path parallel copy with single-source-read optimization.
//!
//! Core responsibilities:
//! - Read source file once, write to multiple destinations simultaneously
//! - Cascading copy (fast device first, then slow devices)
//! - Inline hash verification during copy
//! - Atomic write (.tmp + rename)
//! - Pre-copy space validation

use serde::{Deserialize, Serialize};

/// Status of a single copy task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CopyTaskStatus {
    Pending,
    Copying,
    Verifying,
    Completed,
    Failed(String),
    Skipped,
}

/// A copy task representing one source file to one or more destinations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyTask {
    pub id: String,
    pub source_path: String,
    pub dest_paths: Vec<String>,
    pub file_size: u64,
    pub status: CopyTaskStatus,
}

/// Configuration for the copy engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyEngineConfig {
    /// Buffer size for reading (default: 4MB)
    pub buffer_size: usize,
    /// Maximum retry count on failure
    pub max_retries: u32,
    /// Enable cascading copy
    pub cascading_enabled: bool,
}

impl Default for CopyEngineConfig {
    fn default() -> Self {
        Self {
            buffer_size: 4 * 1024 * 1024, // 4MB
            max_retries: 3,
            cascading_enabled: false,
        }
    }
}
