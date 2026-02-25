//! Volume Management — Storage device tracking and monitoring.
//!
//! Responsibilities:
//! - Auto-discover mounted/unmounted volumes (macOS Disk Arbitration)
//! - Track available space in real-time
//! - Multi-generation copy tracking (which copies live on which volumes)
//! - Last-seen info per volume (who connected it, when)

use serde::{Deserialize, Serialize};

/// Information about a mounted storage volume
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeInfo {
    pub id: String,
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub device_type: DeviceType,
    pub serial_number: Option<String>,
    pub is_mounted: bool,
}

/// Storage device type for IO scheduling
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum DeviceType {
    HDD,
    SSD,
    NVMe,
    RAID,
    Network,
    Unknown,
}
