//! Application Configuration — JSON-persisted user preferences.
//!
//! Stores:
//! - Default hash algorithms
//! - Offload workflow defaults (source verify, post verify, MHL, buffer size)
//! - Per-device-type IO scheduling (concurrency, buffer size)
//! - Email notification (SMTP) settings

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level application settings, persisted as `settings.json`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Offload workflow defaults
    #[serde(default)]
    pub offload: OffloadDefaults,

    /// IO scheduling config per device type
    #[serde(default)]
    pub io_scheduling: IoSchedulingSettings,

    /// Email notification settings
    #[serde(default)]
    pub email: EmailSettings,

    /// Report export settings
    #[serde(default)]
    pub report: ReportSettings,
}

/// Default options for the offload workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OffloadDefaults {
    /// Pre-hash the source before copying
    #[serde(default = "default_true")]
    pub source_verify: bool,

    /// Re-read destination files after copy to verify hashes
    #[serde(default = "default_true")]
    pub post_verify: bool,

    /// Generate ASC MHL manifests after successful copy
    #[serde(default = "default_true")]
    pub generate_mhl: bool,

    /// IO buffer size in bytes (default 4 MB)
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,

    /// Max retry count for failed file copies
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Enable cascading copy: copy to fastest dest first, then cascade
    /// from that copy to slower destinations (frees source card sooner)
    #[serde(default)]
    pub cascade: bool,
}

impl Default for OffloadDefaults {
    fn default() -> Self {
        Self {
            source_verify: true,
            post_verify: true,
            generate_mhl: true,
            buffer_size: default_buffer_size(),
            max_retries: 3,
            cascade: false,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_buffer_size() -> usize {
    4 * 1024 * 1024 // 4 MB
}

fn default_max_retries() -> u32 {
    3
}

/// Per-device-type IO scheduling settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IoSchedulingSettings {
    pub hdd: DeviceIoConfig,
    pub ssd: DeviceIoConfig,
    pub nvme: DeviceIoConfig,
    pub raid: DeviceIoConfig,
    pub network: DeviceIoConfig,
}

impl Default for IoSchedulingSettings {
    fn default() -> Self {
        Self {
            hdd: DeviceIoConfig {
                max_concurrent: 1,
                buffer_size_mb: 1,
            },
            ssd: DeviceIoConfig {
                max_concurrent: 4,
                buffer_size_mb: 4,
            },
            nvme: DeviceIoConfig {
                max_concurrent: 8,
                buffer_size_mb: 8,
            },
            raid: DeviceIoConfig {
                max_concurrent: 4,
                buffer_size_mb: 4,
            },
            network: DeviceIoConfig {
                max_concurrent: 2,
                buffer_size_mb: 1,
            },
        }
    }
}

/// IO config for a single device type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIoConfig {
    pub max_concurrent: usize,
    pub buffer_size_mb: usize,
}

/// Email (SMTP) notification settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSettings {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub smtp_host: String,

    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,

    #[serde(default)]
    pub smtp_username: String,

    /// Note: password is stored separately or left blank for security.
    /// We only persist the flag indicating a password is set.
    #[serde(default)]
    pub smtp_password_set: bool,

    #[serde(default)]
    pub from_address: String,

    #[serde(default)]
    pub to_address: String,

    /// Use STARTTLS
    #[serde(default = "default_true")]
    pub use_tls: bool,
}

impl Default for EmailSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            smtp_host: String::new(),
            smtp_port: default_smtp_port(),
            smtp_username: String::new(),
            smtp_password_set: false,
            from_address: String::new(),
            to_address: String::new(),
            use_tls: true,
        }
    }
}

fn default_smtp_port() -> u16 {
    587
}

/// Report export settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSettings {
    #[serde(default)]
    pub default_export_path: String,

    #[serde(default = "default_export_format")]
    pub export_format: String,

    #[serde(default = "default_true")]
    pub ask_path_each_time: bool,

    #[serde(default)]
    pub ask_format_each_time: bool,
}

impl Default for ReportSettings {
    fn default() -> Self {
        Self {
            default_export_path: String::new(),
            export_format: default_export_format(),
            ask_path_each_time: true,
            ask_format_each_time: false,
        }
    }
}

fn default_export_format() -> String {
    "html".to_string()
}

// ─── Persistence ──────────────────────────────────────────────────────────

const SETTINGS_FILE: &str = "settings.json";

/// Resolve the settings file path inside the given app data directory.
pub fn settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(SETTINGS_FILE)
}

/// Load settings from disk. Returns default settings if file doesn't exist.
pub fn load_settings(app_data_dir: &Path) -> Result<AppSettings> {
    let path = settings_path(app_data_dir);
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read settings from {:?}", path))?;
    let settings: AppSettings =
        serde_json::from_str(&data).with_context(|| "Failed to parse settings JSON")?;
    Ok(settings)
}

/// Save settings to disk (pretty-printed JSON).
pub fn save_settings(app_data_dir: &Path, settings: &AppSettings) -> Result<()> {
    let path = settings_path(app_data_dir);
    let data = serde_json::to_string_pretty(settings).context("Failed to serialize settings")?;
    std::fs::write(&path, data)
        .with_context(|| format!("Failed to write settings to {:?}", path))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_settings() {
        let settings = AppSettings::default();
        assert!(settings.offload.source_verify);
        assert!(settings.offload.post_verify);
        assert!(settings.offload.generate_mhl);
        assert_eq!(settings.offload.buffer_size, 4 * 1024 * 1024);
        assert_eq!(settings.io_scheduling.hdd.max_concurrent, 1);
        assert_eq!(settings.io_scheduling.nvme.max_concurrent, 8);
        assert!(!settings.email.enabled);
    }

    #[test]
    fn test_save_and_load_settings() {
        let tmp = TempDir::new().unwrap();
        let mut settings = AppSettings::default();
        settings.offload.source_verify = false;
        settings.io_scheduling.ssd.max_concurrent = 6;
        settings.email.enabled = true;
        settings.email.smtp_host = "smtp.example.com".to_string();

        save_settings(tmp.path(), &settings).unwrap();
        let loaded = load_settings(tmp.path()).unwrap();

        assert!(!loaded.offload.source_verify);
        assert_eq!(loaded.io_scheduling.ssd.max_concurrent, 6);
        assert!(loaded.email.enabled);
        assert_eq!(loaded.email.smtp_host, "smtp.example.com");
    }

    #[test]
    fn test_load_missing_file_returns_defaults() {
        let tmp = TempDir::new().unwrap();
        let settings = load_settings(tmp.path()).unwrap();
        assert!(settings.offload.source_verify);
        assert_eq!(settings.io_scheduling.hdd.max_concurrent, 1);
    }

    #[test]
    fn test_partial_json_fills_defaults() {
        let tmp = TempDir::new().unwrap();
        let path = settings_path(tmp.path());
        std::fs::write(&path, r#"{"offload": {"sourceVerify": false}}"#).unwrap();

        let settings = load_settings(tmp.path()).unwrap();
        assert!(!settings.offload.source_verify);
        // Defaults should fill in the rest
        assert!(settings.offload.post_verify);
        assert_eq!(settings.io_scheduling.hdd.max_concurrent, 1);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let settings = AppSettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let parsed: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.io_scheduling.nvme.max_concurrent,
            settings.io_scheduling.nvme.max_concurrent
        );
        assert_eq!(parsed.offload.source_verify, settings.offload.source_verify);
    }
}
