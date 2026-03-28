//! Volume Management — Storage device tracking and monitoring.
//!
//! Responsibilities:
//! - Auto-discover mounted/unmounted volumes (macOS /Volumes, Windows drive letters)
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

// ═══════════════════════════════════════════════════════════════════════════
// Unix (macOS / Linux) — Volume space queries
// ═══════════════════════════════════════════════════════════════════════════

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

    // Use u128 for intermediate calculations to prevent overflow with 
    // extremely large drives or weird block sizes.
    let block_size = stat.f_frsize as u128;
    if block_size == 0 {
        anyhow::bail!("Filesystem reported zero fragment size for {:?}", path);
    }

    let total_bytes = (stat.f_blocks as u128) * block_size;
    let free_bytes = (stat.f_bfree as u128) * block_size;
    let available_bytes = (stat.f_bavail as u128) * block_size;
    
    // used_bytes is calculated from total minus free blocks.
    let used_bytes = total_bytes.saturating_sub(free_bytes);
    
    let usage_percent = if total_bytes > 0 {
        (used_bytes as f64 / total_bytes as f64) * 100.0
    } else {
        0.0
    };

    Ok(VolumeSpaceInfo {
        total_bytes: total_bytes as u64,
        available_bytes: available_bytes as u64,
        used_bytes: used_bytes as u64,
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

// ═══════════════════════════════════════════════════════════════════════════
// Windows — Volume space queries via GetDiskFreeSpaceExW
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(windows)]
pub fn get_volume_space(path: &Path) -> Result<VolumeSpaceInfo> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    // Convert path to null-terminated wide string
    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut free_bytes_available: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut total_free_bytes: u64 = 0;

    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide_path.as_ptr()),
            Some(&mut free_bytes_available as *mut u64),
            Some(&mut total_bytes as *mut u64),
            Some(&mut total_free_bytes as *mut u64),
        )
        .context("GetDiskFreeSpaceExW failed")?;
    }

    let used_bytes = total_bytes.saturating_sub(total_free_bytes);
    let usage_percent = if total_bytes > 0 {
        (used_bytes as f64 / total_bytes as f64) * 100.0
    } else {
        0.0
    };

    Ok(VolumeSpaceInfo {
        total_bytes,
        available_bytes: free_bytes_available,
        used_bytes,
        usage_percent,
    })
}

/// Fallback for unsupported platforms
#[cfg(not(any(unix, windows)))]
pub fn get_volume_space(_path: &Path) -> Result<VolumeSpaceInfo> {
    anyhow::bail!("Volume space query not implemented on this platform")
}

// ═══════════════════════════════════════════════════════════════════════════
// macOS — Device type detection via diskutil + mount
// ═══════════════════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════════════════
// Windows — Device type detection via Win32 API + WMI
// ═══════════════════════════════════════════════════════════════════════════

/// Detect device type and file system on Windows.
///
/// Strategy:
/// 1. GetVolumeInformationW → file system name (NTFS, exFAT, FAT32, ReFS)
/// 2. GetDriveTypeW → DRIVE_REMOTE (network), DRIVE_REMOVABLE (SD/USB)
/// 3. WMI MSFT_PhysicalDisk → MediaType (SSD=4, HDD=3), BusType (USB=7, NVMe=17)
#[cfg(windows)]
fn detect_device_info(mount_point: &str) -> (DeviceType, Option<String>) {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW};

    // Ensure mount_point ends with backslash for Win32 APIs (e.g. "C:\")
    let root = if mount_point.ends_with('\\') {
        mount_point.to_string()
    } else {
        format!("{}\\", mount_point)
    };

    let wide_root: Vec<u16> = std::ffi::OsStr::new(&root)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // ── Step 1: Get file system name ──
    let file_system = get_volume_filesystem_win(&wide_root);

    // ── Step 2: Get drive type ──
    let drive_type = unsafe { GetDriveTypeW(PCWSTR(wide_root.as_ptr())) };

    // DRIVE_REMOTE (4) = mapped network drive
    if drive_type == 4 {
        return (DeviceType::Network, file_system);
    }

    // ── Step 3: WMI query for physical disk info (SSD vs HDD) ──
    if let Some(phys) = query_physical_disk_wmi_cached(mount_point) {
        // MediaType: 3 = HDD, 4 = SSD, 5 = SCM
        if phys.media_type == 4 || phys.media_type == 5 {
            return (DeviceType::SSD, file_system);
        }
        if phys.media_type == 3 {
            // HDD, but check if USB removable → might be card reader
            if phys.bus_type == 7 && drive_type == 2 {
                return (DeviceType::SD, file_system);
            }
            return (DeviceType::HDD, file_system);
        }
        // MediaType 0 = Unspecified — check bus type for hints
        if phys.bus_type == 7 && drive_type == 2 {
            // USB + Removable = likely SD card reader
            return (DeviceType::SD, file_system);
        }
        if phys.bus_type == 17 {
            // NVMe = SSD
            return (DeviceType::SSD, file_system);
        }
    }

    // DRIVE_REMOVABLE (2) without WMI info = assume SD/USB card
    if drive_type == 2 {
        return (DeviceType::SD, file_system);
    }

    // DRIVE_FIXED (3) without WMI = assume HDD (conservative)
    if drive_type == 3 {
        return (DeviceType::HDD, file_system);
    }

    (DeviceType::Unknown, file_system)
}

/// Get file system name via GetVolumeInformationW (Windows only).
/// Returns e.g. "NTFS", "exFAT", "FAT32", "ReFS".
#[cfg(windows)]
fn get_volume_filesystem_win(wide_root: &[u16]) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

    let mut fs_name_buf = [0u16; 64];

    let ok = unsafe {
        GetVolumeInformationW(
            PCWSTR(wide_root.as_ptr()),
            None,                       // volume name (we get it separately)
            None,                       // serial number
            None,                       // max component length
            None,                       // file system flags
            Some(&mut fs_name_buf[..]), // file system name
        )
    };

    if ok.is_ok() {
        let len = fs_name_buf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(fs_name_buf.len());
        let name = String::from_utf16_lossy(&fs_name_buf[..len]);
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

/// Get volume label via GetVolumeInformationW (Windows only).
#[cfg(windows)]
fn get_volume_label_win(wide_root: &[u16]) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

    let mut vol_name_buf = [0u16; 256];

    let ok = unsafe {
        GetVolumeInformationW(
            PCWSTR(wide_root.as_ptr()),
            Some(&mut vol_name_buf[..]), // volume name
            None,                        // serial number
            None,                        // max component length
            None,                        // file system flags
            None,                        // file system name
        )
    };

    if ok.is_ok() {
        let len = vol_name_buf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(vol_name_buf.len());
        let name = String::from_utf16_lossy(&vol_name_buf[..len]);
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

/// WMI physical disk info (cached per session).
#[cfg(windows)]
#[derive(Debug, Clone)]
struct PhysicalDiskInfo {
    media_type: u32, // 0=Unspecified, 3=HDD, 4=SSD, 5=SCM
    bus_type: u32,   // 7=USB, 11=SATA, 17=NVMe, etc.
}

/// Query WMI for physical disk info, with caching.
/// Physical disk topology doesn't change during a session, so we cache
/// the mapping of drive letters to physical disk properties.
#[cfg(windows)]
fn query_physical_disk_wmi_cached(mount_point: &str) -> Option<PhysicalDiskInfo> {
    use std::collections::HashMap;
    use std::sync::Mutex;

    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, Option<PhysicalDiskInfo>>>> =
        std::sync::OnceLock::new();

    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());

    // Normalize key: "C:\" → "C:"
    let key = mount_point
        .trim_end_matches('\\')
        .trim_end_matches('/')
        .to_uppercase();

    if let Some(cached) = map.get(&key) {
        return cached.clone();
    }

    // Query WMI (first access only)
    let result = query_physical_disk_wmi_inner(&key);
    map.insert(key, result.clone());
    result
}

/// Inner WMI query for a specific drive letter.
/// Uses `wmic` command as a robust fallback instead of COM/WMI crate
/// to avoid threading issues with COM initialization in async contexts.
#[cfg(windows)]
fn query_physical_disk_wmi_inner(drive_letter: &str) -> Option<PhysicalDiskInfo> {
    use std::process::Command;

    // Use wmic to query physical disk properties for this drive.
    // First, find the disk index for this drive letter.
    // wmic path Win32_LogicalDiskToPartition get Antecedent,Dependent
    // → maps logical disk (C:) to partition, then partition to physical disk.

    // Simpler approach: query Win32_DiskDrive for all disks, then match via
    // Win32_LogicalDiskToPartition. But this is complex with wmic parsing.

    // Simplest reliable approach: use PowerShell one-liner
    let ps_script = format!(
        "Get-Partition -DriveLetter '{}' -ErrorAction SilentlyContinue | \
         Get-Disk | Select-Object -Property MediaType,BusType | \
         ConvertTo-Csv -NoTypeInformation",
        drive_letter.chars().next().unwrap_or('C')
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_script])
        .output()
        .ok()?;

    if !output.status.success() {
        log::debug!(
            "PowerShell disk query failed for {}: {}",
            drive_letter,
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Output is CSV: "MediaType","BusType"\n"SSD","NVMe"\n
    let data_line = stdout.lines().nth(1)?;
    let fields: Vec<&str> = data_line.split(',').map(|f| f.trim_matches('"')).collect();
    if fields.len() < 2 {
        return None;
    }

    let media_type = match fields[0] {
        "HDD" => 3,
        "SSD" => 4,
        "SCM" => 5,
        _ => 0, // Unspecified
    };

    let bus_type = match fields[1] {
        "USB" => 7,
        "SATA" => 11,
        "NVMe" => 17,
        "RAID" => 8,
        "SAS" => 10,
        _ => 0,
    };

    Some(PhysicalDiskInfo {
        media_type,
        bus_type,
    })
}

/// Fallback for non-macOS, non-Windows
#[cfg(not(any(target_os = "macos", windows)))]
fn detect_device_info(_mount_point: &str) -> (DeviceType, Option<String>) {
    (DeviceType::Unknown, None)
}

// ═══════════════════════════════════════════════════════════════════════════
// Volume enumeration — platform dispatch
// ═══════════════════════════════════════════════════════════════════════════

/// List mounted volumes. Dispatches to platform-specific implementation.
pub async fn list_mounted_volumes() -> Result<Vec<VolumeInfo>> {
    #[cfg(target_os = "macos")]
    {
        list_mounted_volumes_macos().await
    }
    #[cfg(windows)]
    {
        list_mounted_volumes_windows().await
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Ok(Vec::new())
    }
}

/// macOS: List mounted volumes by reading /Volumes directory.
#[cfg(target_os = "macos")]
async fn list_mounted_volumes_macos() -> Result<Vec<VolumeInfo>> {
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

/// Windows: List mounted volumes by enumerating drive letters.
#[cfg(windows)]
async fn list_mounted_volumes_windows() -> Result<Vec<VolumeInfo>> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::GetLogicalDriveStringsW;

    // GetLogicalDriveStringsW fills buffer with "A:\\\0C:\\\0D:\\\0\0"
    let mut buffer = vec![0u16; 512];
    let len = unsafe { GetLogicalDriveStringsW(Some(&mut buffer)) };

    if len == 0 {
        anyhow::bail!("GetLogicalDriveStringsW failed");
    }

    let drive_string = String::from_utf16_lossy(&buffer[..len as usize]);
    let mut volumes = Vec::new();

    for drive in drive_string.split('\0').filter(|s| !s.is_empty()) {
        let path = PathBuf::from(drive);

        // Build wide root for Win32 APIs
        let wide_root: Vec<u16> = std::ffi::OsStr::new(drive)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // Volume label (e.g. "Samsung T7", "ALEXA_A001")
        let label = get_volume_label_win(&wide_root);
        let drive_letter = drive.trim_end_matches('\\');
        let name = match &label {
            Some(lbl) if !lbl.is_empty() => format!("{} ({})", lbl, drive_letter),
            _ => drive_letter.to_string(),
        };

        let mount_point = drive.to_string();

        // Get space info
        let (total_bytes, available_bytes) = match get_volume_space(&path) {
            Ok(space) => (space.total_bytes, space.available_bytes),
            Err(e) => {
                // Skip drives that can't be queried (e.g. empty CD-ROM drives)
                log::debug!("Skipping drive {}: {}", drive, e);
                continue;
            }
        };

        // Detect device type and file system
        let (device_type, file_system) = detect_device_info(&mount_point);

        // Deterministic UUID from mount point (same strategy as macOS)
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
/// When `get_volume_space` fails, reports 0 available bytes so the issue surfaces.
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
            // Cannot resolve path at all — report as 0 available
            issues.push((
                dest_path.clone(),
                VolumeSpaceInfo {
                    total_bytes: 0,
                    available_bytes: 0,
                    used_bytes: 0,
                    usage_percent: 0.0,
                },
                *required_bytes,
            ));
            continue;
        };

        match get_volume_space(&check_path) {
            Ok(space) => {
                if !space.has_space_for(*required_bytes) {
                    issues.push((dest_path.clone(), space, *required_bytes));
                }
            }
            Err(e) => {
                log::warn!(
                    "Cannot query space for {:?}: {}, reporting as insufficient",
                    dest_path,
                    e
                );
                issues.push((
                    dest_path.clone(),
                    VolumeSpaceInfo {
                        total_bytes: 0,
                        available_bytes: 0,
                        used_bytes: 0,
                        usage_percent: 0.0,
                    },
                    *required_bytes,
                ));
            }
        }
    }

    issues
}

// ═══════════════════════════════════════════════════════════════════════════
// Disk benchmark — sequential write speed measurement
// ═══════════════════════════════════════════════════════════════════════════

/// Benchmark sequential write speed to a destination path.
/// Writes a 64 MB temporary file in 1 MB chunks with fsync, returns bytes/sec.
/// The temp file is cleaned up afterwards.
pub fn benchmark_write_speed(dest_path: &Path) -> Result<u64> {
    use std::io::Write;

    let dir = if dest_path.is_dir() {
        dest_path.to_path_buf()
    } else {
        dest_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| dest_path.to_path_buf())
    };

    if !dir.exists() {
        anyhow::bail!("Benchmark path does not exist: {:?}", dir);
    }

    let tmp_name = format!(".dit-benchmark-{}.tmp", uuid::Uuid::new_v4());
    let tmp_path = dir.join(&tmp_name);

    const CHUNK_SIZE: usize = 1024 * 1024; // 1 MB
    const TOTAL_SIZE: usize = 64 * 1024 * 1024; // 64 MB
    let chunk = vec![0xA5u8; CHUNK_SIZE];

    let start = std::time::Instant::now();

    let result = (|| -> Result<()> {
        let mut file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("Failed to create benchmark file at {:?}", tmp_path))?;

        let mut written = 0;
        while written < TOTAL_SIZE {
            file.write_all(&chunk)?;
            written += CHUNK_SIZE;
        }
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();

    let elapsed = start.elapsed();

    // Always clean up temp file
    let _ = std::fs::remove_file(&tmp_path);

    result?;

    let secs = elapsed.as_secs_f64();
    if secs <= 0.0 {
        return Ok(TOTAL_SIZE as u64);
    }

    Ok((TOTAL_SIZE as f64 / secs) as u64)
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

    #[cfg(windows)]
    #[test]
    fn test_get_volume_space_c_drive() {
        let space = get_volume_space(Path::new("C:\\")).unwrap();
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

    #[cfg(windows)]
    #[test]
    fn test_preflight_space_check_windows() {
        let temp = std::env::temp_dir();
        let checks = vec![(temp.clone(), 1024), (temp, u64::MAX)];

        let issues = preflight_space_check(&checks);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].2, u64::MAX);
    }
}
