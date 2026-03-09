//! Camera Identification — Detect camera brand/model from folder structure.
//!
//! Scans a source directory for known camera file/folder patterns to identify
//! the camera brand, model hint, reel name, and clip statistics.
//!
//! Supported cameras:
//! - ARRI (`.ari`, `ARRIRAW/`)
//! - RED (`.r3d`, `.RDC/`)
//! - Blackmagic (`.braw`)
//! - Sony (XDCAM/VENICE: `XDROOT/`, `PRIVATE/M4ROOT/`, `.mxf`)
//! - Canon (Cinema EOS: `PRIVATE/AVCHD/`, `CONTENTS/`, `.crm`)
//! - Generic (`.mov`, `.mp4`, etc.)

pub mod metadata;

use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::path::Path;

/// Camera identification result for a source card/folder
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    /// Camera brand ("ARRI", "RED", "Sony", "Blackmagic", "Canon", "Unknown")
    pub brand: String,
    /// Camera model hint if detectable (e.g. from folder naming patterns)
    pub model: String,
    /// Reel/folder name (the source directory name)
    pub reel_name: String,
    /// Number of media clips found
    pub clip_count: u32,
    /// First clip filename (alphabetically)
    pub first_clip: String,
    /// Last clip filename (alphabetically)
    pub last_clip: String,
    /// Total size of all media files in bytes
    pub total_size: u64,
}

impl Default for CameraInfo {
    fn default() -> Self {
        Self {
            brand: "Unknown".to_string(),
            model: String::new(),
            reel_name: String::new(),
            clip_count: 0,
            first_clip: String::new(),
            last_clip: String::new(),
            total_size: 0,
        }
    }
}

/// Known media file extensions for clip counting
const VIDEO_EXTENSIONS: &[&str] = &[
    "ari", "r3d", "braw", "mov", "mp4", "mxf", "crm", "crmz", "mts", "m2ts", "avi", "dpx",
    "exr",
];

/// Identify camera brand and collect clip statistics from a source directory.
///
/// Scans the source path (recursively up to 3 levels) looking for:
/// 1. Known folder structures (ARRIRAW/, .RDC/, XDROOT/, etc.)
/// 2. Known file extensions (.ari, .r3d, .braw, .mxf, .crm)
///
/// Returns a `CameraInfo` with brand, model hint, reel name, and clip stats.
pub fn identify_camera(source_path: &Path) -> CameraInfo {
    let reel_name = source_path
        .file_name()
        .unwrap_or(OsStr::new(""))
        .to_string_lossy()
        .to_string();

    let mut info = CameraInfo {
        reel_name,
        ..Default::default()
    };

    // Collect all media files (recursive, max depth 3)
    let mut media_files: Vec<(String, u64)> = Vec::new(); // (filename, size)
    collect_media_files(source_path, 0, 3, &mut media_files);

    // Count clips and size
    info.clip_count = media_files.len() as u32;
    info.total_size = media_files.iter().map(|(_, s)| s).sum();

    if !media_files.is_empty() {
        // Sort alphabetically for first/last clip
        media_files.sort_by(|a, b| a.0.cmp(&b.0));
        info.first_clip = media_files.first().map(|(n, _)| n.clone()).unwrap_or_default();
        info.last_clip = media_files.last().map(|(n, _)| n.clone()).unwrap_or_default();
    }

    // Detect camera brand from folder structure and file extensions
    detect_brand(source_path, &media_files, &mut info);

    info
}

/// Recursively collect media files up to max_depth levels.
fn collect_media_files(
    dir: &Path,
    current_depth: u32,
    max_depth: u32,
    result: &mut Vec<(String, u64)>,
) {
    if current_depth > max_depth {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with('.') {
                collect_media_files(&path, current_depth + 1, max_depth, result);
            }
        } else if path.is_file() {
            if let Some(ext) = path.extension() {
                let ext_lower = ext.to_string_lossy().to_lowercase();
                if VIDEO_EXTENSIONS.contains(&ext_lower.as_str()) {
                    let file_name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    result.push((file_name, size));
                }
            }
        }
    }
}

/// Detect camera brand from folder structure patterns and file extensions.
fn detect_brand(source_path: &Path, media_files: &[(String, u64)], info: &mut CameraInfo) {
    // Check folder structure patterns first (more reliable)
    if has_subfolder(source_path, "ARRIRAW") || has_subfolder_pattern(source_path, ".ari") {
        info.brand = "ARRI".to_string();
        info.model = detect_arri_model(source_path);
        return;
    }

    if has_rdc_folder(source_path) {
        info.brand = "RED".to_string();
        info.model = detect_red_model(source_path);
        return;
    }

    if has_subfolder(source_path, "XDROOT")
        || has_nested_path(source_path, &["PRIVATE", "M4ROOT"])
        || has_nested_path(source_path, &["XDROOT", "Clip"])
    {
        info.brand = "Sony".to_string();
        info.model = detect_sony_model(source_path);
        return;
    }

    if has_nested_path(source_path, &["PRIVATE", "AVCHD"])
        || has_subfolder(source_path, "CONTENTS")
        || has_subfolder(source_path, "DCIM")
            && has_file_extension_in(source_path, &["crm", "crmz"])
    {
        info.brand = "Canon".to_string();
        return;
    }

    // Fall back to file extension analysis
    let ext_counts = count_extensions(media_files);

    if ext_counts.get("ari").copied().unwrap_or(0) > 0 {
        info.brand = "ARRI".to_string();
        info.model = detect_arri_model(source_path);
    } else if ext_counts.get("r3d").copied().unwrap_or(0) > 0 {
        info.brand = "RED".to_string();
        info.model = detect_red_model(source_path);
    } else if ext_counts.get("braw").copied().unwrap_or(0) > 0 {
        info.brand = "Blackmagic".to_string();
    } else if ext_counts.get("crm").copied().unwrap_or(0) > 0
        || ext_counts.get("crmz").copied().unwrap_or(0) > 0
    {
        info.brand = "Canon".to_string();
    } else if ext_counts.get("mxf").copied().unwrap_or(0) > 0 {
        // MXF could be Sony, ARRI, or generic — check naming patterns
        if media_files.iter().any(|(n, _)| is_sony_naming(n)) {
            info.brand = "Sony".to_string();
        } else {
            info.brand = "Generic".to_string();
        }
    } else if ext_counts.get("mov").copied().unwrap_or(0) > 0
        || ext_counts.get("mp4").copied().unwrap_or(0) > 0
    {
        info.brand = "Generic".to_string();
    }
}

/// Check if source directory contains a subfolder with the given name.
fn has_subfolder(source_path: &Path, name: &str) -> bool {
    source_path.join(name).is_dir()
}

/// Check if any file in the top level has the given extension pattern.
fn has_subfolder_pattern(source_path: &Path, ext: &str) -> bool {
    if let Ok(entries) = std::fs::read_dir(source_path) {
        for entry in entries.flatten() {
            if entry.path().is_file() {
                if let Some(e) = entry.path().extension() {
                    if e.to_string_lossy().to_lowercase() == ext.trim_start_matches('.') {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Check if source has .RDC subfolder (RED Digital Cinema folder).
fn has_rdc_folder(source_path: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(source_path) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".RDC") || name.ends_with(".rdc") {
                    return true;
                }
            }
        }
    }
    false
}

/// Check for a nested directory path (e.g., ["PRIVATE", "M4ROOT"]).
fn has_nested_path(source_path: &Path, parts: &[&str]) -> bool {
    let mut path = source_path.to_path_buf();
    for part in parts {
        path = path.join(part);
    }
    path.is_dir()
}

/// Check if source directory has files with any of the given extensions.
fn has_file_extension_in(source_path: &Path, exts: &[&str]) -> bool {
    if let Ok(entries) = std::fs::read_dir(source_path) {
        for entry in entries.flatten() {
            if entry.path().is_file() {
                if let Some(e) = entry.path().extension() {
                    let ext_lower = e.to_string_lossy().to_lowercase();
                    if exts.contains(&ext_lower.as_str()) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Count file extensions in the media files list.
fn count_extensions(media_files: &[(String, u64)]) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();
    for (name, _) in media_files {
        if let Some(ext) = Path::new(name).extension() {
            let ext_lower = ext.to_string_lossy().to_lowercase();
            *counts.entry(ext_lower).or_insert(0) += 1;
        }
    }
    counts
}

/// Try to detect ARRI model from folder naming conventions.
/// ARRI cards often have patterns like "A001R2EC" (ALEXA naming).
fn detect_arri_model(_source_path: &Path) -> String {
    // ARRI doesn't embed model in folder structure reliably.
    // Would need metadata from .ari headers for specific model.
    String::new()
}

/// Try to detect RED model from .RDC folder naming.
/// RED folders: "A001_C001_0101AB.RDC" — camera letter is first char.
fn detect_red_model(_source_path: &Path) -> String {
    String::new()
}

/// Try to detect Sony model from folder/file naming.
fn detect_sony_model(_source_path: &Path) -> String {
    String::new()
}

/// Check if a filename follows Sony naming patterns (e.g., C0001.MXF, A001C001_xxxxxx.MXF).
fn is_sony_naming(name: &str) -> bool {
    let upper = name.to_uppercase();
    // Sony XDCAM: C0001.MXF, C0002.MXF
    if upper.starts_with('C') && upper.contains(".MXF") {
        if let Some(num_part) = upper.strip_prefix('C') {
            if num_part
                .split('.')
                .next()
                .map(|s| s.chars().all(|c| c.is_ascii_digit()))
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    // Sony VENICE: A001C001_xxxxxx.MXF
    if upper.len() > 8 && upper[1..4].chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    false
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_dir(files: &[&str]) -> TempDir {
        let tmp = TempDir::new().unwrap();
        for file in files {
            let path = tmp.path().join(file);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, "test").unwrap();
        }
        tmp
    }

    #[test]
    fn test_identify_arri_by_extension() {
        let tmp = create_test_dir(&["A001C001.ari", "A001C002.ari", "A001C003.ari"]);
        let info = identify_camera(tmp.path());
        assert_eq!(info.brand, "ARRI");
        assert_eq!(info.clip_count, 3);
        assert_eq!(info.first_clip, "A001C001.ari");
        assert_eq!(info.last_clip, "A001C003.ari");
    }

    #[test]
    fn test_identify_arri_by_folder() {
        let tmp = create_test_dir(&["ARRIRAW/A001C001.ari"]);
        let info = identify_camera(tmp.path());
        assert_eq!(info.brand, "ARRI");
        assert_eq!(info.clip_count, 1);
    }

    #[test]
    fn test_identify_red_by_extension() {
        let tmp = create_test_dir(&["A001_C001_0101.r3d", "A001_C002_0101.r3d"]);
        let info = identify_camera(tmp.path());
        assert_eq!(info.brand, "RED");
        assert_eq!(info.clip_count, 2);
    }

    #[test]
    fn test_identify_red_by_rdc_folder() {
        let tmp = create_test_dir(&["A001_C001_0101AB.RDC/A001_C001_0101AB_001.r3d"]);
        let info = identify_camera(tmp.path());
        assert_eq!(info.brand, "RED");
    }

    #[test]
    fn test_identify_blackmagic() {
        let tmp = create_test_dir(&["clip_001.braw", "clip_002.braw"]);
        let info = identify_camera(tmp.path());
        assert_eq!(info.brand, "Blackmagic");
        assert_eq!(info.clip_count, 2);
    }

    #[test]
    fn test_identify_sony_xdcam() {
        let tmp = create_test_dir(&["XDROOT/Clip/C0001.mxf", "XDROOT/Clip/C0002.mxf"]);
        let info = identify_camera(tmp.path());
        assert_eq!(info.brand, "Sony");
    }

    #[test]
    fn test_identify_sony_venice() {
        let tmp = create_test_dir(&["PRIVATE/M4ROOT/Clip/A001C001_0101.mxf"]);
        let info = identify_camera(tmp.path());
        assert_eq!(info.brand, "Sony");
    }

    #[test]
    fn test_identify_canon() {
        let tmp = create_test_dir(&["PRIVATE/AVCHD/clip.mts"]);
        let info = identify_camera(tmp.path());
        assert_eq!(info.brand, "Canon");
    }

    #[test]
    fn test_identify_generic() {
        let tmp = create_test_dir(&["scene01.mov", "scene02.mov", "scene03.mp4"]);
        let info = identify_camera(tmp.path());
        assert_eq!(info.brand, "Generic");
        assert_eq!(info.clip_count, 3);
    }

    #[test]
    fn test_identify_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let info = identify_camera(tmp.path());
        assert_eq!(info.brand, "Unknown");
        assert_eq!(info.clip_count, 0);
    }

    #[test]
    fn test_reel_name() {
        let tmp = create_test_dir(&["clip.mov"]);
        let info = identify_camera(tmp.path());
        // reel_name should be the directory name
        assert!(!info.reel_name.is_empty());
    }
}
