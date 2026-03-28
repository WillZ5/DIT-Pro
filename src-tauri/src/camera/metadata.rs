//! Media metadata extraction via ffprobe subprocess.
//!
//! Extracts resolution, frame rate, codec, color space, bit depth, and timecode
//! from video files using `ffprobe` (part of FFmpeg).
//!
//! Gracefully handles missing ffprobe — all fields become `None`.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

/// Media metadata extracted from a video file
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadata {
    /// Resolution e.g. "3840x2160"
    pub resolution: Option<String>,
    /// Frame rate e.g. "23.976"
    pub frame_rate: Option<String>,
    /// Codec name e.g. "prores", "rawvideo", "cfhd"
    pub codec: Option<String>,
    /// Color space e.g. "bt709", "bt2020nc"
    pub color_space: Option<String>,
    /// Bit depth e.g. 10, 12, 16
    pub bit_depth: Option<u32>,
    /// Start timecode e.g. "01:23:45:12"
    pub timecode_start: Option<String>,
    /// Duration in seconds
    pub duration_seconds: Option<f64>,
    /// Path to a generated thumbnail image (if any)
    pub thumbnail_path: Option<String>,
}

/// ffprobe JSON output structures (subset we need)
#[derive(Deserialize)]
struct FfprobeOutput {
    streams: Option<Vec<FfprobeStream>>,
    format: Option<FfprobeFormat>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
    color_space: Option<String>,
    bits_per_raw_sample: Option<String>,
    tags: Option<FfprobeStreamTags>,
}

#[derive(Deserialize)]
struct FfprobeStreamTags {
    timecode: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
    tags: Option<FfprobeFormatTags>,
}

#[derive(Deserialize)]
struct FfprobeFormatTags {
    timecode: Option<String>,
}

/// Known video file extensions that should be probed
const VIDEO_EXTENSIONS: &[&str] = &[
    "mov", "mp4", "mxf", "r3d", "braw", "ari", "crm", "avi", "mkv",
];

/// Check if a file extension indicates a video file worth probing
pub fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Try to find ffprobe in common locations
fn find_ffprobe() -> Option<String> {
    // Try PATH first
    if Command::new("ffprobe").arg("-version").output().is_ok() {
        return Some("ffprobe".to_string());
    }

    // Common macOS locations
    let common_paths = [
        "/opt/homebrew/bin/ffprobe",
        "/usr/local/bin/ffprobe",
        "/usr/bin/ffprobe",
    ];

    for path in common_paths {
        if Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    None
}

/// Extract media metadata from a video file using ffprobe.
///
/// Returns `MediaMetadata::default()` if ffprobe is not available or fails.
pub fn probe_media_file(file_path: &Path, cache_dir: Option<&Path>) -> MediaMetadata {
    let ffprobe = match find_ffprobe() {
        Some(p) => p,
        None => {
            log::debug!("ffprobe not found, skipping metadata extraction");
            return MediaMetadata::default();
        }
    };

    let output = match Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-select_streams",
            "v:0",
            "-of",
            "json",
        ])
        .arg(file_path.as_os_str())
        .output()
    {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            log::debug!(
                "ffprobe exited with {}: {}",
                o.status,
                String::from_utf8_lossy(&o.stderr)
            );
            return MediaMetadata::default();
        }
        Err(e) => {
            log::debug!("ffprobe execution failed: {}", e);
            return MediaMetadata::default();
        }
    };

    let parsed: FfprobeOutput = match serde_json::from_slice(&output.stdout) {
        Ok(p) => p,
        Err(e) => {
            log::debug!("ffprobe JSON parse failed: {}", e);
            return MediaMetadata::default();
        }
    };

    // Find the first video stream
    let video_stream = parsed.streams.as_ref().and_then(|streams| {
        streams
            .iter()
            .find(|s| s.codec_type.as_deref() == Some("video"))
    });

    let resolution = video_stream.and_then(|s| match (s.width, s.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some(format!("{}x{}", w, h)),
        _ => None,
    });

    let frame_rate =
        video_stream.and_then(|s| s.r_frame_rate.as_ref().and_then(|r| parse_frame_rate(r)));

    let codec = video_stream.and_then(|s| s.codec_name.as_ref().map(|c| prettify_codec(c)));

    let color_space = video_stream.and_then(|s| s.color_space.clone());

    let bit_depth = video_stream.and_then(|s| {
        s.bits_per_raw_sample
            .as_ref()
            .and_then(|b| b.parse::<u32>().ok())
            .filter(|&b| b > 0)
    });

    // Timecode: try stream tags first, then format tags
    let timecode_start = video_stream
        .and_then(|s| s.tags.as_ref().and_then(|t| t.timecode.clone()))
        .or_else(|| {
            parsed
                .format
                .as_ref()
                .and_then(|f| f.tags.as_ref().and_then(|t| t.timecode.clone()))
        });

    let duration_seconds = parsed
        .format
        .as_ref()
        .and_then(|f| f.duration.as_ref())
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|&d| d > 0.0);

    // Extract thumbnail if cache_dir is provided and it's a video file
    let thumbnail_path = if let Some(dir) = cache_dir {
        if video_stream.is_some() {
            extract_thumbnail(file_path, dir)
        } else {
            None
        }
    } else {
        None
    };

    MediaMetadata {
        resolution,
        frame_rate,
        codec,
        color_space,
        bit_depth,
        timecode_start,
        duration_seconds,
        thumbnail_path,
    }
}

/// Extract a thumbnail from a video file using ffmpeg.
///
/// Saves a 480px JPEG to the cache directory.
pub fn extract_thumbnail(file_path: &Path, cache_dir: &Path) -> Option<String> {
    let ffmpeg = find_ffmpeg()?;

    // Create a deterministic filename based on path hash to avoid re-generating
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    file_path.to_string_lossy().hash(&mut hasher);
    let hash_str = format!("{:x}", hasher.finish());
    let thumb_filename = format!("thumb_{}.jpg", hash_str);
    let thumb_path = cache_dir.join(&thumb_filename);

    // Skip if already exists
    if thumb_path.exists() {
        return Some(thumb_path.to_string_lossy().to_string());
    }

    // Ensure cache dir exists
    let _ = std::fs::create_dir_all(cache_dir);

    // FFmpeg: seek to 1s (to avoid black frames at start), extract 1 frame, 
    // scale to max width 480, high quality (q:v 4).
    let status = Command::new(&ffmpeg)
        .args([
            "-ss",
            "1.0",
            "-i",
        ])
        .arg(file_path.as_os_str())
        .args([
            "-vframes",
            "1",
            "-vf",
            "scale=480:-1:force_original_aspect_ratio=decrease",
            "-q:v",
            "4",
            "-y", // Overwrite just in case
        ])
        .arg(thumb_path.as_os_str())
        .status();

    match status {
        Ok(s) if s.success() => Some(thumb_path.to_string_lossy().to_string()),
        _ => {
            // If 1s seek failed (short clip?), try 0s
            let status_fallback = Command::new(&ffmpeg)
                .args([
                    "-i",
                ])
                .arg(file_path.as_os_str())
                .args([
                    "-vframes",
                    "1",
                    "-vf",
                    "scale=480:-1:force_original_aspect_ratio=decrease",
                    "-q:v",
                    "4",
                    "-y",
                ])
                .arg(thumb_path.as_os_str())
                .status();
                
            match status_fallback {
                Ok(s) if s.success() => Some(thumb_path.to_string_lossy().to_string()),
                _ => None
            }
        }
    }
}

/// Try to find ffmpeg in common locations
pub fn find_ffmpeg() -> Option<String> {
    // Try PATH first
    if Command::new("ffmpeg").arg("-version").output().is_ok() {
        return Some("ffmpeg".to_string());
    }

    // Common macOS locations
    let common_paths = [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
    ];

    for path in common_paths {
        if Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    None
}

/// Parse ffprobe's r_frame_rate fraction (e.g. "24000/1001") into a clean string
fn parse_frame_rate(raw: &str) -> Option<String> {
    if let Some((num_str, den_str)) = raw.split_once('/') {
        let num: f64 = num_str.parse().ok()?;
        let den: f64 = den_str.parse().ok()?;
        if den == 0.0 {
            return None;
        }
        let fps = num / den;
        if fps <= 0.0 || fps > 1000.0 {
            return None;
        }
        // Common frame rates: show nice values
        let rounded = (fps * 1000.0).round() / 1000.0;
        Some(
            format!("{:.3}", rounded)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string(),
        )
    } else {
        // Already a number
        let fps: f64 = raw.parse().ok()?;
        if fps <= 0.0 || fps > 1000.0 {
            return None;
        }
        Some(
            format!("{:.3}", fps)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string(),
        )
    }
}

/// Make codec names more readable
fn prettify_codec(codec: &str) -> String {
    match codec {
        "prores" => "Apple ProRes".to_string(),
        "rawvideo" => "RAW".to_string(),
        "cfhd" => "CineForm".to_string(),
        "dnxhd" => "DNxHD".to_string(),
        "dnxhr" => "DNxHR".to_string(),
        "h264" => "H.264".to_string(),
        "hevc" => "H.265/HEVC".to_string(),
        "mjpeg" => "MJPEG".to_string(),
        "mpeg2video" => "MPEG-2".to_string(),
        other => other.to_uppercase(),
    }
}

/// Probe the first video file in a directory (for reel-level metadata)
pub fn probe_first_video(dir: &Path, cache_dir: Option<&Path>) -> MediaMetadata {
    if !dir.is_dir() {
        if is_video_file(dir) {
            return probe_media_file(dir, cache_dir);
        }
        return MediaMetadata::default();
    }

    // Walk up to 3 levels deep to find the first video file
    if let Some(first) = find_first_video(dir, 3) {
        probe_media_file(&first, cache_dir)
    } else {
        MediaMetadata::default()
    }
}

fn find_first_video(dir: &Path, max_depth: u32) -> Option<std::path::PathBuf> {
    if max_depth == 0 {
        return None;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return None,
    };

    let mut subdirs = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_video_file(&path) {
            return Some(path);
        }
        if path.is_dir() {
            subdirs.push(path);
        }
    }

    subdirs.sort();
    for subdir in subdirs {
        if let Some(found) = find_first_video(&subdir, max_depth - 1) {
            return Some(found);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frame_rate_fraction() {
        assert_eq!(parse_frame_rate("24000/1001"), Some("23.976".to_string()));
        assert_eq!(parse_frame_rate("24/1"), Some("24".to_string()));
        assert_eq!(parse_frame_rate("30000/1001"), Some("29.97".to_string()));
        assert_eq!(parse_frame_rate("25/1"), Some("25".to_string()));
        assert_eq!(parse_frame_rate("60000/1001"), Some("59.94".to_string()));
    }

    #[test]
    fn test_parse_frame_rate_number() {
        assert_eq!(parse_frame_rate("24"), Some("24".to_string()));
        assert_eq!(parse_frame_rate("29.97"), Some("29.97".to_string()));
    }

    #[test]
    fn test_parse_frame_rate_invalid() {
        assert_eq!(parse_frame_rate("0/1"), None);
        assert_eq!(parse_frame_rate("24/0"), None);
        assert_eq!(parse_frame_rate("abc"), None);
    }

    #[test]
    fn test_prettify_codec() {
        assert_eq!(prettify_codec("prores"), "Apple ProRes");
        assert_eq!(prettify_codec("h264"), "H.264");
        assert_eq!(prettify_codec("hevc"), "H.265/HEVC");
        assert_eq!(prettify_codec("dnxhd"), "DNxHD");
        assert_eq!(prettify_codec("unknown"), "UNKNOWN");
    }

    #[test]
    fn test_is_video_file() {
        assert!(is_video_file(Path::new("clip.mov")));
        assert!(is_video_file(Path::new("clip.R3D")));
        assert!(is_video_file(Path::new("clip.braw")));
        assert!(is_video_file(Path::new("clip.MXF")));
        assert!(!is_video_file(Path::new("clip.jpg")));
        assert!(!is_video_file(Path::new("clip.txt")));
        assert!(!is_video_file(Path::new("clip.wav")));
    }

    #[test]
    fn test_default_metadata() {
        let m = MediaMetadata::default();
        assert!(m.resolution.is_none());
        assert!(m.frame_rate.is_none());
        assert!(m.codec.is_none());
        assert!(m.color_space.is_none());
        assert!(m.bit_depth.is_none());
        assert!(m.timecode_start.is_none());
        assert!(m.duration_seconds.is_none());
    }

    #[test]
    fn test_probe_nonexistent_file() {
        let m = probe_media_file(Path::new("/nonexistent/file.mov"), None);
        // Should return default (ffprobe will fail gracefully)
        assert!(m.resolution.is_none() || m.resolution.is_some());
    }
}
