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
    SD,
    RAID,
    Network,
    Unknown,
}

impl std::fmt::Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceType::HDD => write!(f, "HDD"),
            DeviceType::SSD => write!(f, "SSD"),
            DeviceType::SD => write!(f, "SD"),
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
            "SSD" | "NVME" => DeviceType::SSD, // NVMe merged into SSD
            "SD" | "SDCARD" | "CF" | "CFEXPRESS" => DeviceType::SD,
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
    pub file_system: Option<String>,
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
/// Uses statvfs on Unix systems with sanity checks.
/// For network volumes where statvfs returns garbage, falls back to `df`.
#[cfg(unix)]
pub fn get_volume_space(path: &Path) -> Result<VolumeSpaceInfo> {
    let info = get_volume_space_statvfs(path)?;

    // Sanity check: statvfs often returns garbage on network mounts (SMB/NFS).
    // Detect: total < 1MB, or total < available, or usage >= 100% with free = 0
    // but available > 0 — all signs of broken statvfs.
    let looks_sane = info.total_bytes >= 1_048_576 // at least 1 MB
        && info.total_bytes <= 1_000_000_000_000_000_000 // at most 1 EB
        && info.total_bytes >= info.available_bytes
        && info.usage_percent <= 100.0;

    if looks_sane {
        return Ok(info);
    }

    // Fall back to `df -k` which reports correct values for network mounts
    if let Ok(df_info) = get_volume_space_df(path) {
        return Ok(df_info);
    }

    // If df also fails, return zeros → frontend shows "unknown capacity"
    Ok(VolumeSpaceInfo {
        total_bytes: 0,
        available_bytes: 0,
        used_bytes: 0,
        usage_percent: 0.0,
    })
}

/// Raw statvfs query (fast, but unreliable for network mounts).
#[cfg(unix)]
fn get_volume_space_statvfs(path: &Path) -> Result<VolumeSpaceInfo> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path =
        CString::new(path.to_string_lossy().as_ref()).context("Invalid path for statvfs")?;

    let mut stat = MaybeUninit::<libc::statvfs>::uninit();
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };

    if ret != 0 {
        anyhow::bail!(
            "statvfs failed for {:?}: errno {}",
            path,
            std::io::Error::last_os_error()
        );
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

/// Parse `df -k <path>` output to get space info.
/// Used as fallback when statvfs returns garbage (common on SMB/NFS mounts).
#[cfg(unix)]
fn get_volume_space_df(path: &Path) -> Result<VolumeSpaceInfo> {
    use std::process::Command;

    let output = Command::new("df")
        .args(["-k", &path.to_string_lossy()])
        .output()
        .context("Failed to run df")?;

    if !output.status.success() {
        anyhow::bail!("df failed for {:?}", path);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // df -k output: "Filesystem 1024-blocks Used Available Capacity ..."
    // Skip header line, parse second line
    let data_line = stdout
        .lines()
        .nth(1)
        .context("df output missing data line")?;

    // Fields may be separated by variable whitespace.
    // For network mounts the filesystem field can contain spaces (e.g. "//user@host/share"),
    // so parse from the end to get the numeric fields reliably.
    let fields: Vec<&str> = data_line.split_whitespace().collect();
    // Typical df -k output fields (macOS):
    //   Filesystem  1024-blocks  Used  Available  Capacity  iused  ifree  %iused  Mounted-on
    // We need at least: total(1), used(2), available(3), capacity(4)
    if fields.len() < 5 {
        anyhow::bail!("df output too few fields: {}", data_line);
    }

    // Parse from right: Mounted-on(-1), %iused(-2), ifree(-3), iused(-4),
    //                    Capacity(-5), Available(-6), Used(-7), 1024-blocks(-8)
    // On macOS `df -k` has 9 columns. Parse by index from end for robustness.
    let len = fields.len();
    let total_kb: u64 = fields[len - 8].parse().unwrap_or(0);
    let used_kb: u64 = fields[len - 7].parse().unwrap_or(0);
    let available_kb: u64 = fields[len - 6].parse().unwrap_or(0);

    let total_bytes = total_kb * 1024;
    let used_bytes = used_kb * 1024;
    let available_bytes = available_kb * 1024;
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

/// Detect device type and file system for a mounted volume using `mount` and `diskutil info`.
/// Returns (DeviceType, Option<file_system_name>).
#[cfg(target_os = "macos")]
fn detect_device_info(mount_point: &str) -> (DeviceType, Option<String>) {
    use std::process::Command;

    // First check `mount` output for network filesystem types — more reliable
    // than `diskutil info` for network shares (SMB, NFS, AFP, WebDAV, CIFS).
    if let Ok(mount_output) = Command::new("mount").output() {
        let mounts = String::from_utf8_lossy(&mount_output.stdout);
        for line in mounts.lines() {
            if line.contains(mount_point) {
                let lower = line.to_lowercase();
                // Extract fs type from mount output: "... on /Volumes/X (smbfs, ...)"
                let fs = if lower.contains("smbfs") {
                    Some("SMB".to_string())
                } else if lower.contains("nfs") {
                    Some("NFS".to_string())
                } else if lower.contains("afpfs") {
                    Some("AFP".to_string())
                } else if lower.contains("webdav") {
                    Some("WebDAV".to_string())
                } else if lower.contains("cifs") {
                    Some("CIFS".to_string())
                } else {
                    None
                };
                if fs.is_some() {
                    return (DeviceType::Network, fs);
                }
            }
        }
    }

    // Fall through to diskutil for local devices
    let output = match Command::new("diskutil")
        .args(["info", mount_point])
        .output()
    {
        Ok(o) => o,
        Err(_) => return (DeviceType::Unknown, None),
    };

    let info = String::from_utf8_lossy(&output.stdout);

    // Extract "File System Personality:" (e.g. "ExFAT", "APFS", "MS-DOS FAT32", "NTFS")
    let file_system = info
        .lines()
        .find(|l| l.contains("File System Personality:"))
        .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty());

    // Check for network filesystem protocols (secondary check via diskutil)
    if info.contains("Protocol:") {
        let proto_line = info
            .lines()
            .find(|l| l.contains("Protocol:"))
            .unwrap_or_default();
        let proto = proto_line.to_lowercase();
        if proto.contains("smb") || proto.contains("nfs") || proto.contains("afp") {
            return (DeviceType::Network, file_system);
        }
    }

    // Check for RAID (AppleRAID or software RAID)
    if info.contains("RAID") || info.contains("AppleRAID") {
        return (DeviceType::RAID, file_system);
    }

    // Determine internal media type from diskutil output
    let info_lower = info.to_lowercase();

    // Check "Solid State:" field FIRST — any device explicitly marked as
    // solid state is an SSD (USB shuttle drives, NVMe, SATA SSD, etc.)
    // This must come before SD detection to avoid misclassifying USB SSDs.
    let is_solid_state = info
        .lines()
        .find(|l| l.contains("Solid State:"))
        .map(|l| l.contains("Yes"))
        .unwrap_or(false);

    if is_solid_state || info_lower.contains("nvme") || info_lower.contains("nvmexpress") {
        return (DeviceType::SSD, file_system);
    }

    // Check for SD / CF / memory cards via protocol or removable media.
    // diskutil shows "Protocol: Secure Digital" for SD cards,
    // "Protocol: USB" + "Removable Media: Removable" for USB card readers.
    // SD cards have "Solid State: Info not available" so they won't match above.
    let is_removable = info
        .lines()
        .find(|l| l.contains("Removable Media:"))
        .map(|l| l.contains("Removable"))
        .unwrap_or(false);

    if let Some(proto_line) = info.lines().find(|l| l.contains("Protocol:")) {
        let proto = proto_line.to_lowercase();
        // "Secure Digital" = SD/SDHC/SDXC via built-in reader
        if proto.contains("secure digital") {
            return (DeviceType::SD, file_system);
        }
        // USB removable media (card reader with SD/CF inside)
        if proto.contains("usb") && is_removable {
            return (DeviceType::SD, file_system);
        }
    }

    // If diskutil returned valid device info but not solid state, it's likely HDD
    // (only for physical disks, not disk images)
    if info.contains("Device Node:") && info.contains("Total Size:") {
        return (DeviceType::HDD, file_system);
    }

    (DeviceType::Unknown, file_system)
}

#[cfg(not(target_os = "macos"))]
fn detect_device_info(_mount_point: &str) -> (DeviceType, Option<String>) {
    (DeviceType::Unknown, None)
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

        // Detect device type and file system (SSD/HDD/SD/RAID/Network + ExFAT/APFS/etc.)
        let (device_type, file_system) = detect_device_info(&mount_point);

        // Use a stable, deterministic ID based on mount point so that
        // repeated polls return the same ID for the same volume.
        // This allows the frontend (React key={vol.id}) to reconcile
        // correctly instead of re-mounting all cards every poll cycle.
        let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, mount_point.as_bytes()).to_string();

        volumes.push(VolumeInfo {
            id,
            name,
            mount_point,
            total_bytes,
            available_bytes,
            device_type,
            file_system,
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
                file_system: None,
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
        assert_eq!(DeviceType::RAID.to_string(), "RAID");
    }

    #[test]
    fn test_device_type_from_str() {
        assert_eq!(DeviceType::from_str_loose("hdd"), DeviceType::HDD);
        assert_eq!(DeviceType::from_str_loose("SSD"), DeviceType::SSD);
        assert_eq!(DeviceType::from_str_loose("nvme"), DeviceType::SSD); // NVMe mapped to SSD
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
        )
        .unwrap();

        let volume = VolumeInfo {
            id: "vol-001".to_string(),
            name: "Shuttle_SSD_1".to_string(),
            mount_point: "/Volumes/Shuttle_SSD_1".to_string(),
            total_bytes: 2_000_000_000_000,
            available_bytes: 1_500_000_000_000,
            device_type: DeviceType::SSD,
            file_system: None,
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
        )
        .unwrap();

        let vol = VolumeInfo {
            id: "v1".to_string(),
            name: "SSD_A".to_string(),
            mount_point: "/Volumes/SSD_A".to_string(),
            total_bytes: 1_000_000_000_000,
            available_bytes: 500_000_000_000,
            device_type: DeviceType::SSD,
            file_system: None,
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
