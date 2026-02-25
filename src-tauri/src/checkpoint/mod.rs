//! Checkpoint / Recovery System — Crash-safe task state persistence.
//!
//! Uses SQLite WAL mode to maintain task state across crashes.
//! State machine: pending → copying → verifying → completed | failed
//! Recovery flow:
//! 1. Scan tasks with status != completed
//! 2. Clean up .tmp files
//! 3. Reset interrupted tasks to pending
//! 4. Resume from last completed file

use serde::{Deserialize, Serialize};

/// Persistent task state for checkpoint recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub task_id: String,
    pub source_path: String,
    pub dest_path: String,
    pub status: String,
    pub file_hash: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
