//! IO Scheduler — Per-drive concurrency control and smart queue management.
//!
//! Default concurrency limits:
//! - HDD: 1-2 (protect from excessive seek)
//! - SSD (SATA): 4
//! - NVMe: 8
//! - RAID: 4-8 (based on RAID level)
//! - Network: 2-4
//!
//! Each device gets an independent task queue and semaphore.
//! Slow devices don't block fast devices.

use serde::{Deserialize, Serialize};

use crate::volume::DeviceType;

/// Per-device IO scheduling configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSchedulerConfig {
    pub device_type: DeviceType,
    pub max_concurrent_tasks: usize,
}

impl DeviceSchedulerConfig {
    pub fn default_for(device_type: DeviceType) -> Self {
        let max_concurrent = match device_type {
            DeviceType::HDD => 1,
            DeviceType::SSD => 4,
            DeviceType::NVMe => 8,
            DeviceType::RAID => 4,
            DeviceType::Network => 2,
            DeviceType::Unknown => 2,
        };
        Self {
            device_type,
            max_concurrent_tasks: max_concurrent,
        }
    }
}
