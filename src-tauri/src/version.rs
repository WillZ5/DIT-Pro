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

// ─── Update Checker ─────────────────────────────────────────────────────────

/// Result of checking for a newer release on GitHub / Gitee.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    /// Whether a newer version is available
    pub has_update: bool,
    /// Latest version tag (e.g. "v1.0.2")
    pub latest_version: String,
    /// Current running version
    pub current_version: String,
    /// Release notes / body (Markdown)
    pub release_notes: String,
    /// Direct URL to the release page
    pub release_url: String,
    /// Download URL for the DMG asset (if found)
    pub download_url: Option<String>,
    /// Published date
    pub published_at: String,
}

/// Minimal JSON shape returned by GitHub / Gitee Releases API.
#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
    assets: Option<Vec<GhAsset>>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Compare two semver strings (e.g. "1.0.1" vs "1.0.2").
/// Returns true if `remote` is newer than `local`.
fn is_newer(local: &str, remote: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .filter_map(|p| p.parse::<u64>().ok())
            .collect()
    };
    let l = parse(local);
    let r = parse(remote);
    for i in 0..3 {
        let lv = l.get(i).copied().unwrap_or(0);
        let rv = r.get(i).copied().unwrap_or(0);
        if rv > lv {
            return true;
        }
        if rv < lv {
            return false;
        }
    }
    false
}

const GITHUB_API: &str = "https://api.github.com/repos/WillZ5/DIT-Pro/releases/latest";
const WEBSITE_LATEST: &str = "https://ditpro.negdims.com/software/latest.json";

/// Shape of the website's `/software/latest.json`.
#[derive(Debug, Deserialize)]
struct WebsiteLatest {
    tag_name: String,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
    download_url: Option<String>,
}

impl From<WebsiteLatest> for GhRelease {
    fn from(w: WebsiteLatest) -> Self {
        let assets = w.download_url.map(|url| {
            vec![GhAsset {
                name: url.rsplit('/').next().unwrap_or("DIT-Pro.dmg").to_string(),
                browser_download_url: url,
            }]
        });
        GhRelease {
            tag_name: w.tag_name,
            body: w.body,
            html_url: w.html_url,
            published_at: w.published_at,
            assets,
        }
    }
}

/// Fetch JSON from a URL with timeout + retries. Deserializes into `T`.
async fn fetch_json<T: serde::de::DeserializeOwned>(
    url: &str,
    retries: u32,
    timeout_secs: u64,
) -> Result<T, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let mut last_err = String::new();
    for attempt in 1..=retries {
        match client
            .get(url)
            .header("User-Agent", "DIT-Pro-Updater")
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => match resp.json::<T>().await {
                Ok(data) => return Ok(data),
                Err(e) => last_err = format!("JSON parse error: {}", e),
            },
            Ok(resp) => {
                last_err = format!("HTTP {}", resp.status());
            }
            Err(e) => {
                last_err = format!("Request failed: {}", e);
            }
        }
        if attempt < retries {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    Err(last_err)
}

/// Check for updates: GitHub first (10s timeout, 3 retries), fallback to website.
pub async fn check_for_update() -> Result<UpdateCheckResult, String> {
    let current = env!("CARGO_PKG_VERSION");

    // Try GitHub API first
    let release = match fetch_json::<GhRelease>(GITHUB_API, 3, 10).await {
        Ok(r) => {
            log::info!("Update check: fetched from GitHub (tag={})", r.tag_name);
            r
        }
        Err(gh_err) => {
            log::warn!("GitHub update check failed: {}, trying website...", gh_err);
            // Fallback to website latest.json
            match fetch_json::<WebsiteLatest>(WEBSITE_LATEST, 3, 10).await {
                Ok(w) => {
                    log::info!("Update check: fetched from website (tag={})", w.tag_name);
                    GhRelease::from(w)
                }
                Err(web_err) => {
                    return Err(format!(
                        "Update check failed — GitHub: {}; Website: {}",
                        gh_err, web_err
                    ));
                }
            }
        }
    };

    let remote_ver = release.tag_name.trim_start_matches('v');
    let has_update = is_newer(current, remote_ver);

    // Find DMG asset
    let download_url = release.assets.as_ref().and_then(|assets| {
        assets
            .iter()
            .find(|a| a.name.ends_with(".dmg"))
            .map(|a| a.browser_download_url.clone())
    });

    Ok(UpdateCheckResult {
        has_update,
        latest_version: release.tag_name,
        current_version: format!("v{}", current),
        release_notes: release.body.unwrap_or_default(),
        release_url: release.html_url,
        download_url,
        published_at: release.published_at.unwrap_or_default(),
    })
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
