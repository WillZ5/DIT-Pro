//! Version Management for DIT Pro.
//!
//! Provides structured version information with SemVer compliance,
//! pre-release tags (alpha/beta/rc), and build metadata (git hash + date).
//!
//! Build-time environment variables (set by build.rs):
//!   DIT_GIT_HASH     — short git commit hash
//!   DIT_BUILD_TIME   — build date (YYYYMMDD UTC)
//!   DIT_PRE_RELEASE  — pre-release tag (e.g., "alpha.1", "beta.2", "rc.1")

use serde::{Deserialize, Serialize};

// ─── Release Channel ─────────────────────────────────────────────────────────

/// Release channel derived from version and pre-release tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseChannel {
    /// Local development (0.x without pre-release tag)
    Dev,
    /// Internal testing
    Alpha,
    /// External / expanded testing
    Beta,
    /// Release candidate
    Rc,
    /// Production release (1.0.0+)
    Stable,
}

impl std::fmt::Display for ReleaseChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dev => write!(f, "dev"),
            Self::Alpha => write!(f, "alpha"),
            Self::Beta => write!(f, "beta"),
            Self::Rc => write!(f, "rc"),
            Self::Stable => write!(f, "stable"),
        }
    }
}

// ─── VersionInfo ─────────────────────────────────────────────────────────────

/// Complete version information for the application.
///
/// Full format: `MAJOR.MINOR.PATCH[-prerelease][+buildmeta]`
///
/// Examples:
///   - `0.5.0` (dev)
///   - `0.6.0-alpha.1` (alpha)
///   - `1.0.0-rc.1+20260225.a3b4c5d` (RC with build metadata)
///   - `1.0.0` (stable release)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    /// Base SemVer version: "0.5.0"
    pub version: String,
    /// Pre-release tag: "alpha.1", "beta.2", "rc.1", or None for stable/dev
    pub pre_release: Option<String>,
    /// Build metadata: "20260225.a3b4c5d"
    pub build_meta: Option<String>,
    /// Release channel
    pub channel: ReleaseChannel,
    /// Full version string: "0.5.0-alpha.1+20260225.a3b4c5d"
    pub full_string: String,
    /// Git commit short hash (7 chars)
    pub git_hash: Option<String>,
    /// Build timestamp (YYYYMMDD UTC)
    pub build_time: Option<String>,
}

impl VersionInfo {
    /// Build VersionInfo from compile-time environment variables.
    pub fn current() -> Self {
        let version = env!("CARGO_PKG_VERSION").to_string();
        let pre_release = option_env!("DIT_PRE_RELEASE").map(|s| s.to_string());
        let git_hash = option_env!("DIT_GIT_HASH").map(|s| s.to_string());
        let build_time = option_env!("DIT_BUILD_TIME").map(|s| s.to_string());

        let channel = Self::derive_channel(&version, pre_release.as_deref());

        let build_meta = match (&build_time, &git_hash) {
            (Some(time), Some(hash)) => Some(format!("{}.{}", time, hash)),
            (None, Some(hash)) => Some(hash.clone()),
            (Some(time), None) => Some(time.clone()),
            (None, None) => None,
        };

        let full_string =
            Self::format_full(&version, pre_release.as_deref(), build_meta.as_deref());

        Self {
            version,
            pre_release,
            build_meta,
            channel,
            full_string,
            git_hash,
            build_time,
        }
    }

    /// Derive release channel from version and pre-release tag.
    fn derive_channel(version: &str, pre_release: Option<&str>) -> ReleaseChannel {
        match pre_release {
            Some(p) if p.starts_with("alpha") => ReleaseChannel::Alpha,
            Some(p) if p.starts_with("beta") => ReleaseChannel::Beta,
            Some(p) if p.starts_with("rc") => ReleaseChannel::Rc,
            Some(_) => ReleaseChannel::Dev,
            None => {
                if version.starts_with("0.") {
                    ReleaseChannel::Dev
                } else {
                    ReleaseChannel::Stable
                }
            }
        }
    }

    /// Format the full version string.
    fn format_full(version: &str, pre_release: Option<&str>, build_meta: Option<&str>) -> String {
        let mut s = version.to_string();
        if let Some(pre) = pre_release {
            s.push('-');
            s.push_str(pre);
        }
        if let Some(meta) = build_meta {
            s.push('+');
            s.push_str(meta);
        }
        s
    }
}

impl Default for VersionInfo {
    fn default() -> Self {
        Self::current()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_version() {
        let info = VersionInfo::current();
        assert!(!info.version.is_empty());
        assert!(info.full_string.starts_with(&info.version));
    }

    #[test]
    fn test_channel_detection() {
        assert_eq!(
            VersionInfo::derive_channel("0.5.0", None),
            ReleaseChannel::Dev
        );
        assert_eq!(
            VersionInfo::derive_channel("1.0.0", None),
            ReleaseChannel::Stable
        );
        assert_eq!(
            VersionInfo::derive_channel("0.6.0", Some("alpha.1")),
            ReleaseChannel::Alpha
        );
        assert_eq!(
            VersionInfo::derive_channel("0.6.0", Some("beta.2")),
            ReleaseChannel::Beta
        );
        assert_eq!(
            VersionInfo::derive_channel("1.0.0", Some("rc.1")),
            ReleaseChannel::Rc
        );
        assert_eq!(
            VersionInfo::derive_channel("0.5.0", Some("dev")),
            ReleaseChannel::Dev
        );
    }

    #[test]
    fn test_full_string_format() {
        assert_eq!(VersionInfo::format_full("1.0.0", None, None), "1.0.0");
        assert_eq!(
            VersionInfo::format_full("0.6.0", Some("alpha.1"), None),
            "0.6.0-alpha.1"
        );
        assert_eq!(
            VersionInfo::format_full("1.0.0", Some("rc.1"), Some("20260225.a3b4c5d")),
            "1.0.0-rc.1+20260225.a3b4c5d"
        );
        assert_eq!(
            VersionInfo::format_full("1.0.0", None, Some("20260225.a3b4c5d")),
            "1.0.0+20260225.a3b4c5d"
        );
    }

    #[test]
    fn test_channel_display() {
        assert_eq!(ReleaseChannel::Alpha.to_string(), "alpha");
        assert_eq!(ReleaseChannel::Stable.to_string(), "stable");
    }

    #[test]
    fn test_version_serialization() {
        let info = VersionInfo::current();
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"version\""));
        assert!(json.contains("\"channel\""));
        assert!(json.contains("\"fullString\""));

        // Roundtrip
        let deserialized: VersionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.version, info.version);
    }
}
