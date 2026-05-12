//! Proxy Generation — Transcode high-res footage to editing-friendly proxies.
//!
//! Uses `ffmpeg` to generate low-bitrate versions of original clips.
//! Supports H.264/AAC and ProRes Proxy.

use super::metadata::find_ffmpeg;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProxyFormat {
    H264,
    ProResProxy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    pub format: ProxyFormat,
    pub width: u32,
    pub burn_timecode: bool,
    pub crf: u32, // For H.264
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            format: ProxyFormat::H264,
            width: 1920,
            burn_timecode: true,
            crf: 24,
        }
    }
}

fn escape_drawtext_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' | ':' | '\'' | ',' | '[' | ']' | '%' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn ffmpeg_supports_filter(ffmpeg: &str, filter_name: &str) -> bool {
    let output = Command::new(ffmpeg)
        .args(["-hide_banner", "-filters"])
        .output();

    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .any(|line| line.split_whitespace().any(|token| token == filter_name))
}

/// Transcode a single video file to a proxy file.
pub fn generate_proxy(
    source: &Path,
    dest_dir: &Path,
    config: &ProxyConfig,
    timecode: Option<&str>,
) -> Result<PathBuf> {
    let ffmpeg = find_ffmpeg().context("FFmpeg not found")?;

    // Ensure dest dir exists
    std::fs::create_dir_all(dest_dir)?;

    let file_stem = source
        .file_stem()
        .context("Invalid filename")?
        .to_string_lossy();
    let ext = match config.format {
        ProxyFormat::H264 => "mp4",
        ProxyFormat::ProResProxy => "mov",
    };

    let dest_path = dest_dir.join(format!("{}_proxy.{}", file_stem, ext));

    let mut args = vec![
        "-hide_banner".to_string(),
        "-y".to_string(),
        "-i".to_string(),
        source.to_string_lossy().to_string(),
        // Camera MOV files often contain timecode/data streams that MP4 cannot
        // mux. Map only the primary video and optional audio tracks.
        "-map".to_string(),
        "0:v:0".to_string(),
        "-map".to_string(),
        "0:a?".to_string(),
        "-dn".to_string(),
        "-sn".to_string(),
    ];

    // Scale filter
    let mut vf = format!("scale={}:-1", config.width);

    // Timecode burn-in filter
    if config.burn_timecode {
        if let Some(tc) = timecode {
            if ffmpeg_supports_filter(&ffmpeg, "drawtext") {
                #[cfg(target_os = "macos")]
                let font = "/System/Library/Fonts/Helvetica.ttc";
                #[cfg(target_os = "windows")]
                let font = "C\\\\:/Windows/Fonts/arial.ttf";
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                let font = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

                let escaped_font = escape_drawtext_value(font);
                let escaped_text = escape_drawtext_value(file_stem.as_ref());
                let escaped_timecode = escape_drawtext_value(tc);

                vf = format!(
                    "{},drawtext=fontfile='{}':text='{}':timecode='{}':rate=24:x=(w-tw)/2:y=h-th-20:fontcolor=white:fontsize=48:box=1:boxcolor=black@0.5",
                    vf, escaped_font, escaped_text, escaped_timecode
                );
            } else {
                log::warn!(
                    "FFmpeg drawtext filter unavailable; generating proxy without burned-in timecode for {:?}",
                    source
                );
            }
        }
    }

    args.push("-vf".to_string());
    args.push(vf);

    // Codec specific args
    match config.format {
        ProxyFormat::H264 => {
            args.extend_from_slice(&[
                "-c:v".to_string(),
                "libx264".to_string(),
                "-preset".to_string(),
                "fast".to_string(),
                "-crf".to_string(),
                config.crf.to_string(),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "128k".to_string(),
                "-pix_fmt".to_string(),
                "yuv420p".to_string(),
            ]);
        }
        ProxyFormat::ProResProxy => {
            args.extend_from_slice(&[
                "-c:v".to_string(),
                "prores_ks".to_string(),
                "-profile:v".to_string(),
                "0".to_string(), // 0 = proxy
                "-c:a".to_string(),
                "pcm_s16le".to_string(),
            ]);
        }
    }

    args.push(dest_path.to_string_lossy().to_string());

    let output = Command::new(ffmpeg).args(&args).output()?;

    if output.status.success() {
        Ok(dest_path)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "FFmpeg transcode failed for {:?}: {}",
            source,
            stderr.trim()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_config_accepts_frontend_camel_case() {
        let config: ProxyConfig =
            serde_json::from_str(r#"{"format":"H264","width":1280,"burnTimecode":false,"crf":26}"#)
                .expect("frontend proxy config should deserialize");

        assert!(matches!(config.format, ProxyFormat::H264));
        assert_eq!(config.width, 1280);
        assert!(!config.burn_timecode);
        assert_eq!(config.crf, 26);

        let value = serde_json::to_value(&config).expect("proxy config should serialize");
        assert_eq!(value["burnTimecode"], false);
        assert!(value.get("burn_timecode").is_none());
    }

    #[test]
    fn drawtext_escaping_handles_timecode_and_paths() {
        assert_eq!(escape_drawtext_value("01:00:00:00"), "01\\:00\\:00\\:00");
        assert_eq!(
            escape_drawtext_value("C\\:/Windows/Fonts/arial.ttf"),
            "C\\\\\\:/Windows/Fonts/arial.ttf"
        );
        assert_eq!(
            escape_drawtext_value("clip's [A], 50%"),
            "clip\\'s \\[A\\]\\, 50\\%"
        );
    }
}
