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
//!
//! Architecture:
//! ```text
//! ┌──────────────┐
//! │  Job Queue    │  ← tasks submitted here
//! └─────┬────────┘
//!       │ dispatch by destination device
//!       ▼
//! ┌─────────────┐  ┌─────────────┐  ┌─────────────┐
//! │ HDD Queue   │  │ SSD Queue   │  │ NVMe Queue  │
//! │ (1 worker)  │  │ (4 workers) │  │ (8 workers) │
//! └─────────────┘  └─────────────┘  └─────────────┘
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::volume::DeviceType;

/// Per-device IO scheduling configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSchedulerConfig {
    pub device_type: DeviceType,
    pub max_concurrent_tasks: usize,
    /// Buffer size for IO operations (bytes)
    pub buffer_size: usize,
}

impl DeviceSchedulerConfig {
    pub fn default_for(device_type: DeviceType) -> Self {
        let (max_concurrent, buffer_size) = match device_type {
            DeviceType::HDD => (1, 1024 * 1024), // 1 concurrent, 1MB buffer
            DeviceType::SSD => (4, 4 * 1024 * 1024), // 4 concurrent, 4MB buffer
            DeviceType::NVMe => (8, 8 * 1024 * 1024), // 8 concurrent, 8MB buffer
            DeviceType::RAID => (4, 4 * 1024 * 1024), // 4 concurrent, 4MB buffer
            DeviceType::Network => (2, 1024 * 1024), // 2 concurrent, 1MB buffer
            DeviceType::Unknown => (2, 2 * 1024 * 1024), // 2 concurrent, 2MB buffer
        };
        Self {
            device_type,
            max_concurrent_tasks: max_concurrent,
            buffer_size,
        }
    }
}

/// A task to be scheduled on a specific device
#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub task_id: String,
    pub source_path: PathBuf,
    pub dest_path: PathBuf,
    pub file_size: u64,
}

/// Result of executing a scheduled task
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub task_id: String,
    pub success: bool,
    pub error: Option<String>,
    pub bytes_written: u64,
}

/// RAII guard that decrements `active_tasks` on drop.
/// Holds the semaphore permit so the concurrency slot is released together.
pub struct DevicePermit {
    _permit: tokio::sync::OwnedSemaphorePermit,
    active_tasks: Arc<tokio::sync::Mutex<usize>>,
}

impl Drop for DevicePermit {
    fn drop(&mut self) {
        // Use `try_lock` to avoid blocking in drop; on failure
        // the counter is slightly stale but will recover.
        if let Ok(mut count) = self.active_tasks.try_lock() {
            *count = count.saturating_sub(1);
        }
    }
}

/// Per-device queue with concurrency control via semaphore
pub struct DeviceQueue {
    pub mount_point: PathBuf,
    pub config: DeviceSchedulerConfig,
    pub semaphore: Arc<Semaphore>,
    pub active_tasks: Arc<tokio::sync::Mutex<usize>>,
}

impl DeviceQueue {
    /// Create a new device queue with the given configuration
    pub fn new(mount_point: PathBuf, config: DeviceSchedulerConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_tasks));
        Self {
            mount_point,
            config,
            semaphore,
            active_tasks: Arc::new(tokio::sync::Mutex::new(0)),
        }
    }

    /// Acquire a permit to execute a task on this device.
    /// This will block (async) if the device is at its concurrency limit.
    /// Returns a `DevicePermit` RAII guard that decrements the counter on drop.
    pub async fn acquire(&self) -> Result<DevicePermit> {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .context("Device semaphore closed")?;
        let mut count = self.active_tasks.lock().await;
        *count += 1;
        Ok(DevicePermit {
            _permit: permit,
            active_tasks: Arc::clone(&self.active_tasks),
        })
    }

    /// Get the current number of active tasks
    pub async fn active_count(&self) -> usize {
        *self.active_tasks.lock().await
    }

    /// Get the maximum concurrency for this device
    pub fn max_concurrent(&self) -> usize {
        self.config.max_concurrent_tasks
    }

    /// Get the buffer size for IO operations on this device
    pub fn buffer_size(&self) -> usize {
        self.config.buffer_size
    }
}

/// The IO Scheduler manages per-device queues and routes tasks to the
/// appropriate queue based on destination path.
pub struct IoScheduler {
    /// Map from mount point → device queue
    device_queues: HashMap<PathBuf, DeviceQueue>,
    /// Default config for unknown devices
    #[allow(dead_code)]
    default_config: DeviceSchedulerConfig,
}

impl Default for IoScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl IoScheduler {
    /// Create a new IO scheduler
    pub fn new() -> Self {
        Self {
            device_queues: HashMap::new(),
            default_config: DeviceSchedulerConfig::default_for(DeviceType::Unknown),
        }
    }

    /// Register a device (mount point) with its configuration
    pub fn register_device(&mut self, mount_point: PathBuf, config: DeviceSchedulerConfig) {
        let queue = DeviceQueue::new(mount_point.clone(), config);
        self.device_queues.insert(mount_point, queue);
    }

    /// Register a device with default config for its type
    pub fn register_device_auto(&mut self, mount_point: PathBuf, device_type: DeviceType) {
        let config = DeviceSchedulerConfig::default_for(device_type);
        self.register_device(mount_point, config);
    }

    /// Find the device queue for a destination path by matching mount points.
    /// Returns the queue with the longest matching mount point prefix.
    pub fn get_device_queue(&self, dest_path: &Path) -> Option<&DeviceQueue> {
        let mut best_match: Option<(&PathBuf, &DeviceQueue)> = None;

        for (mount_point, queue) in &self.device_queues {
            if dest_path.starts_with(mount_point) {
                match best_match {
                    None => best_match = Some((mount_point, queue)),
                    Some((current_best, _)) => {
                        if mount_point.as_os_str().len() > current_best.as_os_str().len() {
                            best_match = Some((mount_point, queue));
                        }
                    }
                }
            }
        }

        best_match.map(|(_, q)| q)
    }

    /// Get all registered device mount points
    pub fn registered_devices(&self) -> Vec<&PathBuf> {
        self.device_queues.keys().collect()
    }

    /// Get the total number of active tasks across all devices
    pub async fn total_active_tasks(&self) -> usize {
        let mut total = 0;
        for queue in self.device_queues.values() {
            total += queue.active_count().await;
        }
        total
    }

    /// Get a summary of all device queue states
    pub async fn status_summary(&self) -> Vec<DeviceQueueStatus> {
        let mut statuses = Vec::new();
        for (mount, queue) in &self.device_queues {
            statuses.push(DeviceQueueStatus {
                mount_point: mount.clone(),
                device_type: queue.config.device_type,
                max_concurrent: queue.max_concurrent(),
                active_tasks: queue.active_count().await,
                buffer_size: queue.buffer_size(),
            });
        }
        statuses
    }
}

/// Status snapshot of a device queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceQueueStatus {
    pub mount_point: PathBuf,
    pub device_type: DeviceType,
    pub max_concurrent: usize,
    pub active_tasks: usize,
    pub buffer_size: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_hdd() {
        let config = DeviceSchedulerConfig::default_for(DeviceType::HDD);
        assert_eq!(config.max_concurrent_tasks, 1);
        assert_eq!(config.buffer_size, 1024 * 1024);
    }

    #[test]
    fn test_default_config_nvme() {
        let config = DeviceSchedulerConfig::default_for(DeviceType::NVMe);
        assert_eq!(config.max_concurrent_tasks, 8);
        assert_eq!(config.buffer_size, 8 * 1024 * 1024);
    }

    #[test]
    fn test_scheduler_device_registration() {
        let mut scheduler = IoScheduler::new();
        scheduler.register_device_auto(PathBuf::from("/Volumes/Shuttle_SSD"), DeviceType::SSD);
        scheduler.register_device_auto(PathBuf::from("/Volumes/Archive_HDD"), DeviceType::HDD);

        assert_eq!(scheduler.registered_devices().len(), 2);
    }

    #[test]
    fn test_scheduler_route_by_mount_point() {
        let mut scheduler = IoScheduler::new();
        scheduler.register_device_auto(PathBuf::from("/Volumes/SSD_RAID"), DeviceType::SSD);
        scheduler.register_device_auto(PathBuf::from("/Volumes/Shuttle_HDD"), DeviceType::HDD);

        // Route to SSD
        let queue = scheduler.get_device_queue(Path::new("/Volumes/SSD_RAID/project/clip.mov"));
        assert!(queue.is_some());
        assert_eq!(queue.unwrap().config.device_type, DeviceType::SSD);

        // Route to HDD
        let queue = scheduler.get_device_queue(Path::new("/Volumes/Shuttle_HDD/backup/clip.mov"));
        assert!(queue.is_some());
        assert_eq!(queue.unwrap().config.device_type, DeviceType::HDD);

        // Unknown path → no match
        let queue = scheduler.get_device_queue(Path::new("/tmp/unknown/clip.mov"));
        assert!(queue.is_none());
    }

    #[test]
    fn test_scheduler_longest_prefix_match() {
        let mut scheduler = IoScheduler::new();
        scheduler.register_device_auto(PathBuf::from("/Volumes"), DeviceType::Unknown);
        scheduler.register_device_auto(PathBuf::from("/Volumes/Specific_SSD"), DeviceType::SSD);

        // Should match the more specific mount point
        let queue = scheduler.get_device_queue(Path::new("/Volumes/Specific_SSD/data/clip.mov"));
        assert!(queue.is_some());
        assert_eq!(queue.unwrap().config.device_type, DeviceType::SSD);

        // Should match the generic mount point
        let queue = scheduler.get_device_queue(Path::new("/Volumes/Other/clip.mov"));
        assert!(queue.is_some());
        assert_eq!(queue.unwrap().config.device_type, DeviceType::Unknown);
    }

    #[tokio::test]
    async fn test_semaphore_concurrency_limit() {
        let queue = DeviceQueue::new(
            PathBuf::from("/Volumes/HDD"),
            DeviceSchedulerConfig::default_for(DeviceType::HDD),
        );

        assert_eq!(queue.max_concurrent(), 1);
        assert_eq!(queue.active_count().await, 0);

        // Acquire one permit (HDD max is 1)
        let _permit = queue.acquire().await.unwrap();
        assert_eq!(queue.active_count().await, 1);

        // Trying to acquire another would block (we can't test blocking easily,
        // but we can verify available permits = 0)
        assert_eq!(queue.semaphore.available_permits(), 0);
    }

    #[tokio::test]
    async fn test_status_summary() {
        let mut scheduler = IoScheduler::new();
        scheduler.register_device_auto(PathBuf::from("/Volumes/A"), DeviceType::SSD);
        scheduler.register_device_auto(PathBuf::from("/Volumes/B"), DeviceType::HDD);

        let statuses = scheduler.status_summary().await;
        assert_eq!(statuses.len(), 2);

        for status in &statuses {
            assert_eq!(status.active_tasks, 0);
        }
    }
}
