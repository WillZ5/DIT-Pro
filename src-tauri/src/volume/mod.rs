//! Volume Management — Storage device tracking and monitoring.
//!
//! Responsibilities:
//! - Auto-discover mounted/unmounted volumes (macOS Disk Arbitration)
//! - Track available space in real-time
//! - Multi-generation copy tracking (which copies live on which volumes)
//! - Last-seen info per volume (who connected it, when)

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

impl std::fmt::Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceType::HDD => write!(f, "HDD"),
            DeviceType::SSD => write!(f, "SSD"),
            DeviceType::NVMe => write!(f, "NVMe"),
            DeviceType::RAID => write!(f, "RAID"),
            DeviceType::Network => write!(f, "Network"),
            DeviceType::Unknown => write!(f, "Unknown"),
        }
    }
}

impl DeviceType {
    /// Parse from string (database storage)
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "HDD" => DeviceType::HDD,
            "SSD" => DeviceType::SSD,
            "NVME" => DeviceType::NVMe,
            "RAID" => DeviceType::RAID,
            "NETWORK" => DeviceType::Network,
            _ => DeviceType::Unknown,
        }
    }
}

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
    pub last_seen_at: Option<DateTime<Utc>>,
    pub last_seen_by: Option<String>,
}

/// Space usage summary for a volume
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSpaceInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub usage_percent: f64,
}

impl VolumeSpaceInfo {
    /// Check if there's enough space for a given file size
    pub fn has_space_for(&self, required_bytes: u64) -> bool {
        self.available_bytes >= required_bytes
    }

    /// Check if the volume is critically low on space (< 1 GB)
    pub fn is_critically_low(&self) -> bool {
        self.available_bytes < 1_073_741_824 // 1 GB
    }

    /// Check if the volume is low on space (< 10 GB)
    pub fn is_low(&self) -> bool {
        self.available_bytes < 10_737_418_240 // 10 GB
    }
}

/// Query the filesystem for space information on a path.
/// Uses statvfs on Unix systems.
#[cfg(unix)]
pub fn get_volume_space(path: &Path) -> Result<VolumeSpaceInfo> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = CString::new(path.to_string_lossy().as_ref())
        .context("Invalid path for statvfs")?;

    let mut stat = MaybeUninit::<libc::statvfs>::uninit();
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };

    if ret != 0 {
        anyhow::bail!("statvfs failed for {:?}: errno {}", path, std::io::Error::last_os_error());
    }

    let stat = unsafe { stat.assume_init() };
    let block_size = stat.f_frsize;
    let total_bytes = (stat.f_blocks as u64) * block_size;
    let available_bytes = (stat.f_bavail as u64) * block_size;
    let used_bytes = total_bytes.saturating_sub((stat.f_bfree as u64) * block_size);
    let usage_percent = if total_bytes > 0 {
        (used_bytes as f64 / total_bytes as f64) * 100.0
    } else {
        0.0
    };

    Ok(VolumeSpaceInfo {
        total_bytes,
        available_bytes,
        used_bytes,
        usage_percent,
    })
}

#[cfg(not(unix))]
pub fn get_volume_space(_path: &Path) -> Result<VolumeSpaceInfo> {
    anyhow::bail!("Volume space query not implemented on this platform")
}

/// Detect device type for a mounted volume using `diskutil info`.
#[cfg(target_os = "macos")]
fn detect_device_type(mount_point: &str) -> DeviceType {
    use std::process::Command;

    // Network shares (SMB, NFS, AFP) are not under /dev
    let output = match Command::new("diskutil")
        .args(["info", mount_point])
        .output()
    {
        Ok(o) => o,
        Err(_) => return DeviceType::Unknown,
    };

    let info = String::from_utf8_lossy(&output.stdout);

    // Check for network filesystem protocols
    if info.contains("Protocol:") {
        let proto_line = info.lines().find(|l| l.contains("Protocol:")).unwrap_or_default();
        let proto = proto_line.to_lowercase();
        if proto.contains("smb") || proto.contains("nfs") || proto.contains("afp") {
            return DeviceType::Network;
        }
    }

    // Check for RAID (AppleRAID or software RAID)
    if info.contains("RAID") || info.contains("AppleRAID") {
        return DeviceType::RAID;
    }

    // Determine internal media type from diskutil output
    let info_lower = info.to_lowercase();

    // Check "Solid State:" field  (Yes = SSD/NVMe)
    let is_solid_state = info.lines()
        .find(|l| l.contains("Solid State:"))
        .map(|l| l.contains("Yes"))
        .unwrap_or(false);

    // Check protocol/bus for NVMe
    if info_lower.contains("nvme") || info_lower.contains("nvmexpress") {
        return DeviceType::NVMe;
    }

    if is_solid_state {
        return DeviceType::SSD;
    }

    // If diskutil returned valid device info but not solid state, it's likely HDD
    // (only for physical disks, not disk images)
    if info.contains("Device Node:") && info.contains("Total Size:") {
        return DeviceType::HDD;
    }

    DeviceType::Unknown
}

#[cfg(not(target_os = "macos"))]
fn detect_device_type(_mount_point: &str) -> DeviceType {
    DeviceType::Unknown
}

/// List mounted volumes on macOS by reading /Volumes directory.
pub async fn list_mounted_volumes() -> Result<Vec<VolumeInfo>> {
    let volumes_dir = Path::new("/Volumes");
    if !volumes_dir.exists() {
        return Ok(Vec::new());
    }

    let mut volumes = Vec::new();
    let mut entries = tokio::fs::read_dir(volumes_dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        let mount_point = path.to_string_lossy().to_string();

        // Get space info
        let (total_bytes, available_bytes) = match get_volume_space(&path) {
            Ok(space) => (space.total_bytes, space.available_bytes),
            Err(_) => (0, 0),
        };

        // Detect device type (SSD/NVMe/HDD/RAID/Network)
        let device_type = detect_device_type(&mount_point);

        let id = uuid::Uuid::new_v4().to_string();

        volumes.push(VolumeInfo {
            id,
            name,
            mount_point,
            total_bytes,
            available_bytes,
            device_type,
            serial_number: None,
            is_mounted: true,
            last_seen_at: Some(Utc::now()),
            last_seen_by: None,
        });
    }

    Ok(volumes)
}

// ─── Database Operations ─────────────────────────────────────────────────

/// Save or update a volume record in the database
pub fn upsert_volume(conn: &Connection, volume: &VolumeInfo) -> Result<()> {
    conn.execute(
        "INSERT INTO volumes (id, name, mount_point, total_bytes, available_bytes,
         device_type, serial_number, last_seen_at, last_seen_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
         name = excluded.name,
         mount_point = excluded.mount_point,
         total_bytes = excluded.total_bytes,
         available_bytes = excluded.available_bytes,
         device_type = excluded.device_type,
         serial_number = excluded.serial_number,
         last_seen_at = excluded.last_seen_at,
         last_seen_by = excluded.last_seen_by",
        params![
            volume.id,
            volume.name,
            volume.mount_point,
            volume.total_bytes as i64,
            volume.available_bytes as i64,
            volume.device_type.to_string(),
            volume.serial_number,
            volume.last_seen_at.map(|dt| dt.to_rfc3339()),
            volume.last_seen_by,
        ],
    )?;
    Ok(())
}

/// Get all known volumes from the database
pub fn get_all_volumes(conn: &Connection) -> Result<Vec<VolumeInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, mount_point, total_bytes, available_bytes,
                device_type, serial_number, last_seen_at, last_seen_by
         FROM volumes ORDER BY name ASC",
    )?;

    let volumes = stmt
        .query_map([], |row| {
            let dt_str: Option<String> = row.get(7)?;
            let last_seen_at = dt_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            });

            Ok(VolumeInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                mount_point: row.get(2)?,
                total_bytes: row.get::<_, i64>(3)? as u64,
                available_bytes: row.get::<_, i64>(4)? as u64,
                device_type: DeviceType::from_str_loose(&row.get::<_, String>(5)?),
                serial_number: row.get(6)?,
                is_mounted: false,
                last_seen_at,
                last_seen_by: row.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to read volumes")?;

    Ok(volumes)
}

/// Find a volume by mount point
pub fn find_volume_by_mount(conn: &Connection, mount_point: &str) -> Result<Option<VolumeInfo>> {
    let volumes = get_all_volumes(conn)?;
    Ok(volumes.into_iter().find(|v| v.mount_point == mount_point))
}

/// Pre-flight check: validate that all destination paths have sufficient space.
/// Returns a list of (path, space_info, required_bytes) for paths with issues.
pub fn preflight_space_check(
    destinations: &[(PathBuf, u64)],
) -> Vec<(PathBuf, VolumeSpaceInfo, u64)> {
    let mut issues = Vec::new();

    for (dest_path, required_bytes) in destinations {
        let check_path = if dest_path.exists() {
            dest_path.clone()
        } else if let Some(parent) = dest_path.parent() {
            parent.to_path_buf()
        } else {
            continue;
        };

        if let Ok(space) = get_volume_space(&check_path) {
            if !space.has_space_for(*required_bytes) {
                issues.push((dest_path.clone(), space, *required_bytes));
            }
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_type_display() {
        assert_eq!(DeviceType::HDD.to_string(), "HDD");
        assert_eq!(DeviceType::SSD.to_string(), "SSD");
        assert_eq!(DeviceType::NVMe.to_string(), "NVMe");
    }

    #[test]
    fn test_device_type_from_str() {
        assert_eq!(DeviceType::from_str_loose("hdd"), DeviceType::HDD);
        assert_eq!(DeviceType::from_str_loose("SSD"), DeviceType::SSD);
        assert_eq!(DeviceType::from_str_loose("nvme"), DeviceType::NVMe);
        assert_eq!(DeviceType::from_str_loose("garbage"), DeviceType::Unknown);
    }

    #[test]
    fn test_space_info_checks() {
        let space = VolumeSpaceInfo {
            total_bytes: 1_000_000_000_000,
            available_bytes: 500_000_000_000,
            used_bytes: 500_000_000_000,
            usage_percent: 50.0,
        };

        assert!(space.has_space_for(100_000_000_000));
        assert!(!space.has_space_for(600_000_000_000));
        assert!(!space.is_low());
        assert!(!space.is_critically_low());
    }

    #[test]
    fn test_space_info_critically_low() {
        let space = VolumeSpaceInfo {
            total_bytes: 1_000_000_000_000,
            available_bytes: 500_000_000,
            used_bytes: 999_500_000_000,
            usage_percent: 99.95,
        };

        assert!(space.is_critically_low());
        assert!(space.is_low());
    }

    #[cfg(unix)]
    #[test]
    fn test_get_volume_space_current_dir() {
        let space = get_volume_space(Path::new("/")).unwrap();
        assert!(space.total_bytes > 0);
        assert!(space.available_bytes > 0);
        assert!(space.usage_percent > 0.0);
        assert!(space.usage_percent <= 100.0);
    }

    #[test]
    fn test_volume_db_operations() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS volumes (
                id TEXT PRIMARY KEY, name TEXT NOT NULL,
                mount_point TEXT, total_bytes INTEGER,
                available_bytes INTEGER, device_type TEXT,
                serial_number TEXT, last_seen_at TEXT, last_seen_by TEXT
            );",
        ).unwrap();

        let volume = VolumeInfo {
            id: "vol-001".to_string(),
            name: "Shuttle_SSD_1".to_string(),
            mount_point: "/Volumes/Shuttle_SSD_1".to_string(),
            total_bytes: 2_000_000_000_000,
            available_bytes: 1_500_000_000_000,
            device_type: DeviceType::SSD,
            serial_number: Some("SN12345".to_string()),
            is_mounted: true,
            last_seen_at: Some(Utc::now()),
            last_seen_by: Some("DIT Operator".to_string()),
        };

        upsert_volume(&conn, &volume).unwrap();

        let volumes = get_all_volumes(&conn).unwrap();
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].name, "Shuttle_SSD_1");
        assert_eq!(volumes[0].device_type, DeviceType::SSD);
        assert_eq!(volumes[0].serial_number.as_deref(), Some("SN12345"));
        assert_eq!(volumes[0].total_bytes, 2_000_000_000_000);

        // Upsert update
        let mut updated = volume.clone();
        updated.available_bytes = 1_000_000_000_000;
        upsert_volume(&conn, &updated).unwrap();

        let volumes = get_all_volumes(&conn).unwrap();
        assert_eq!(volumes[0].available_bytes, 1_000_000_000_000);
    }

    #[test]
    fn test_find_volume_by_mount() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS volumes (
                id TEXT PRIMARY KEY, name TEXT NOT NULL,
                mount_point TEXT, total_bytes INTEGER,
                available_bytes INTEGER, device_type TEXT,
                serial_number TEXT, last_seen_at TEXT, last_seen_by TEXT
            );",
        ).unwrap();

        let vol = VolumeInfo {
            id: "v1".to_string(),
            name: "SSD_A".to_string(),
            mount_point: "/Volumes/SSD_A".to_string(),
            total_bytes: 1_000_000_000_000,
            available_bytes: 500_000_000_000,
            device_type: DeviceType::SSD,
            serial_number: None,
            is_mounted: true,
            last_seen_at: None,
            last_seen_by: None,
        };

        upsert_volume(&conn, &vol).unwrap();

        let found = find_volume_by_mount(&conn, "/Volumes/SSD_A").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "SSD_A");

        let not_found = find_volume_by_mount(&conn, "/Volumes/Nonexistent").unwrap();
        assert!(not_found.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn test_preflight_space_check() {
        let checks = vec![
            (PathBuf::from("/tmp"), 1024),
            (PathBuf::from("/tmp"), u64::MAX),
        ];

        let issues = preflight_space_check(&checks);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].2, u64::MAX);
    }
}
