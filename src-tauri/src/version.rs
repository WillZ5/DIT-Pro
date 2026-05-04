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
        let (version, embedded_pre_release) = Self::split_pre_release(env!("CARGO_PKG_VERSION"));
        let pre_release = option_env!("DIT_PRE_RELEASE")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or(embedded_pre_release);
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
        let (_, embedded_pre_release) = Self::split_pre_release(version);
        let effective_pre_release = pre_release.or(embedded_pre_release.as_deref());

        match effective_pre_release {
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
        let (base_version, embedded_pre_release) = Self::split_pre_release(version);
        let effective_pre_release = pre_release.or(embedded_pre_release.as_deref());
        let mut s = base_version;
        if let Some(pre) = effective_pre_release {
            s.push('-');
            s.push_str(pre);
        }
        if let Some(meta) = build_meta {
            s.push('+');
            s.push_str(meta);
        }
        s
    }

    fn split_pre_release(raw_version: &str) -> (String, Option<String>) {
        let version_without_meta = raw_version
            .split_once('+')
            .map_or(raw_version, |(base, _)| base);
        match version_without_meta.split_once('-') {
            Some((base, pre)) if !pre.is_empty() => (base.to_string(), Some(pre.to_string())),
            _ => (version_without_meta.to_string(), None),
        }
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

/// Compare two semver strings (e.g. "1.0.1" vs "1.0.2").
/// Returns true if `remote` is newer than `local`.
fn is_newer(local: &str, remote: &str) -> bool {
    let parse = |s: &str| -> ([u64; 3], Option<String>) {
        let without_v = s.trim().trim_start_matches('v');
        let without_build = without_v
            .split_once('+')
            .map_or(without_v, |(base, _)| base);
        let (core, pre) = without_build
            .split_once('-')
            .map_or((without_build, None), |(core, pre)| {
                (core, Some(pre.to_string()))
            });
        let mut parts = [0_u64; 3];
        for (idx, part) in core.split('.').take(3).enumerate() {
            parts[idx] = part.parse::<u64>().unwrap_or(0);
        }
        (parts, pre)
    };
    let (l, l_pre) = parse(local);
    let (r, r_pre) = parse(remote);
    for i in 0..3 {
        let lv = l[i];
        let rv = r[i];
        if rv > lv {
            return true;
        }
        if rv < lv {
            return false;
        }
    }
    match (l_pre, r_pre) {
        (Some(_), None) => true,
        (None, Some(_)) | (None, None) => false,
        (Some(local_pre), Some(remote_pre)) => remote_pre > local_pre,
    }
}

const WEBSITE_HOME: &str = "https://ditpro.negdims.com/";
const WEBSITE_LATEST: &str = "https://ditpro.negdims.com/software/latest.json";

/// Shape of the website's `/software/latest.json` (fallback source).
/// Extra fields kept for forward compatibility with the JSON schema.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct WebsiteLatest {
    tag_name: String,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
    download_url: Option<String>,
}

fn normalize_version_tag(version: &str) -> String {
    let version = version.trim();
    if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{}", version)
    }
}

/// Build an HTTP client with timeout.
fn build_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))
}

/// Fetch raw text from a URL with retries.
async fn fetch_text(url: &str, retries: u32, timeout_secs: u64) -> Result<String, String> {
    let client = build_client(timeout_secs)?;
    let mut last_err = String::new();
    for attempt in 1..=retries {
        match client
            .get(url)
            .header("User-Agent", "DIT-Pro-Updater")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => match resp.text().await {
                Ok(text) => return Ok(text),
                Err(e) => last_err = format!("Body read error: {}", e),
            },
            Ok(resp) => last_err = format!("HTTP {}", resp.status()),
            Err(e) => last_err = format!("Request failed: {}", e),
        }
        if attempt < retries {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    Err(last_err)
}

/// Fetch JSON from a URL with retries. Deserializes into `T`.
async fn fetch_json<T: serde::de::DeserializeOwned>(
    url: &str,
    retries: u32,
    timeout_secs: u64,
) -> Result<T, String> {
    let client = build_client(timeout_secs)?;
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
            Ok(resp) => last_err = format!("HTTP {}", resp.status()),
            Err(e) => last_err = format!("Request failed: {}", e),
        }
        if attempt < retries {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    Err(last_err)
}

/// Parse `<meta name="ditpro-stable-version" content="vX.X.X">` from HTML.
/// Returns the content value (e.g. "v1.2.0") or None.
fn parse_stable_version_meta(html: &str) -> Option<String> {
    fn attr_value(tag: &str, attr: &str) -> Option<String> {
        for quote in ['"', '\''] {
            let marker = format!("{}={}", attr, quote);
            if let Some(start) = tag.find(&marker) {
                let value_start = start + marker.len();
                let value_end = tag[value_start..].find(quote)? + value_start;
                return Some(tag[value_start..value_end].trim().to_string());
            }
        }
        None
    }

    let mut offset = 0;
    while let Some(meta_start_rel) = html[offset..].find("<meta") {
        let meta_start = offset + meta_start_rel;
        let Some(meta_end_rel) = html[meta_start..].find('>') else {
            break;
        };
        let meta_end = meta_start + meta_end_rel;
        let tag = &html[meta_start..=meta_end];
        if attr_value(tag, "name").as_deref() == Some("ditpro-stable-version") {
            let version = attr_value(tag, "content")?;
            if is_stable_version_tag(&version) {
                return Some(version);
            }
            return None;
        }
        offset = meta_end + 1;
    }
    None
}

fn is_stable_version_tag(version: &str) -> bool {
    // Validate format: must be vX.X.X
    version.starts_with('v')
        && version[1..].split('.').count() == 3
        && version[1..].split('.').all(|p| p.parse::<u64>().is_ok())
}

/// Shared update check implementation with injectable endpoints for tests.
async fn check_for_update_from_sources(
    current: &str,
    website_home: &str,
    website_latest: &str,
) -> Result<UpdateCheckResult, String> {
    // ── Primary: stable manifest ──
    let manifest = match fetch_json::<WebsiteLatest>(website_latest, 3, 10).await {
        Ok(w) => {
            log::info!(
                "Update check: fetched from latest.json (tag={})",
                w.tag_name
            );
            Some(w)
        }
        Err(json_err) => {
            log::warn!(
                "Update check: latest.json failed: {}, trying website meta fallback...",
                json_err
            );
            None
        }
    };

    // ── Fallback: hidden website meta tag ──
    let (final_version, final_notes, final_download, final_published) = if let Some(w) = manifest {
        (
            normalize_version_tag(&w.tag_name),
            w.body.unwrap_or_default(),
            w.download_url,
            w.published_at.unwrap_or_default(),
        )
    } else {
        let html = fetch_text(website_home, 3, 10).await?;
        let stable_version = parse_stable_version_meta(&html).ok_or_else(|| {
            "Update check failed: latest.json unavailable and website meta tag not found"
                .to_string()
        })?;
        log::info!(
            "Update check: parsed stable version from website meta: {}",
            stable_version
        );
        (stable_version, String::new(), None, String::new())
    };

    let remote_ver = final_version.trim_start_matches('v');
    let has_update = is_newer(current, remote_ver);

    Ok(UpdateCheckResult {
        has_update,
        latest_version: final_version,
        current_version: normalize_version_tag(current),
        release_notes: final_notes,
        release_url: website_home.to_string(),
        download_url: final_download,
        published_at: final_published,
    })
}

/// Check for updates:
/// 1. Primary: fetch `/software/latest.json`
/// 2. Fallback: fetch website HTML, parse `<meta name="ditpro-stable-version">`
pub async fn check_for_update() -> Result<UpdateCheckResult, String> {
    let full = VersionInfo::current().full_string;
    let current = full
        .split_once('+')
        .map_or_else(|| full.clone(), |(base, _)| base.to_string());
    check_for_update_from_sources(&current, WEBSITE_HOME, WEBSITE_LATEST).await
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Clone)]
    struct TestResponse {
        status: &'static str,
        content_type: &'static str,
        body: String,
    }

    async fn spawn_test_server(
        routes: HashMap<String, TestResponse>,
    ) -> (String, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let routes = Arc::new(routes);
        let request_log = Arc::clone(&requests);

        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let routes = Arc::clone(&routes);
                let request_log = Arc::clone(&request_log);

                tokio::spawn(async move {
                    let mut buf = [0_u8; 4096];
                    let n = match stream.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => return,
                    };

                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .to_string();

                    request_log.lock().unwrap().push(path.clone());

                    let response = routes.get(&path).cloned().unwrap_or(TestResponse {
                        status: "404 Not Found",
                        content_type: "text/plain",
                        body: "not found".to_string(),
                    });

                    let http = format!(
                        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.status,
                        response.content_type,
                        response.body.len(),
                        response.body
                    );

                    let _ = stream.write_all(http.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });

        (base_url, requests, handle)
    }

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
        assert_eq!(
            VersionInfo::format_full("1.4.0-beta", Some("beta.3"), None),
            "1.4.0-beta.3"
        );
    }

    #[test]
    fn test_embedded_pre_release_normalization() {
        assert_eq!(
            VersionInfo::split_pre_release("1.4.0-beta"),
            ("1.4.0".to_string(), Some("beta".to_string()))
        );
        assert_eq!(
            VersionInfo::derive_channel("1.4.0-beta", None),
            ReleaseChannel::Beta
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

    #[test]
    fn test_parse_stable_version_meta() {
        // Normal case
        let html = r#"<html><head>
            <meta name="ditpro-stable-version" content="v1.2.0">
        </head></html>"#;
        assert_eq!(parse_stable_version_meta(html), Some("v1.2.0".to_string()));

        // Different version
        let html2 = r#"<meta name="ditpro-stable-version" content="v2.0.1">"#;
        assert_eq!(parse_stable_version_meta(html2), Some("v2.0.1".to_string()));

        // Missing meta tag
        let html3 = r#"<html><head><meta name="description" content="test"></head></html>"#;
        assert_eq!(parse_stable_version_meta(html3), None);

        // Invalid version format (has beta suffix)
        let html4 = r#"<meta name="ditpro-stable-version" content="v1.3.0-beta">"#;
        assert_eq!(parse_stable_version_meta(html4), None);

        // Missing v prefix
        let html5 = r#"<meta name="ditpro-stable-version" content="1.2.0">"#;
        assert_eq!(parse_stable_version_meta(html5), None);

        // content before name attribute (reversed order)
        let html6 = r#"<meta content="v1.5.0" name="ditpro-stable-version">"#;
        assert_eq!(parse_stable_version_meta(html6), Some("v1.5.0".to_string()));

        // single quotes
        let html7 = r#"<meta name='ditpro-stable-version' content='v1.6.0'>"#;
        assert_eq!(parse_stable_version_meta(html7), Some("v1.6.0".to_string()));
    }

    #[tokio::test]
    async fn test_check_for_update_prefers_manifest_and_returns_website_url() {
        let mut routes = HashMap::new();
        routes.insert(
            "/".to_string(),
            TestResponse {
                status: "200 OK",
                content_type: "text/html",
                body: r#"
                    <html><head>
                      <meta name="ditpro-stable-version" content="v1.2.0">
                    </head><body>DIT Pro</body></html>
                "#
                .to_string(),
            },
        );
        routes.insert(
            "/software/latest.json".to_string(),
            TestResponse {
                status: "200 OK",
                content_type: "application/json",
                body: r#"{
                    "tag_name":"v1.3.0",
                    "body":"manifest notes",
                    "html_url":"https://example.com/releases/v1.3.0",
                    "published_at":"2026-03-10T00:00:00Z",
                    "download_url":"https://example.com/dit-pro.dmg"
                }"#
                .to_string(),
            },
        );

        let (base_url, request_log, server) = spawn_test_server(routes).await;
        let home_url = format!("{}/", base_url);
        let latest_url = format!("{}/software/latest.json", base_url);

        let result = check_for_update_from_sources("1.1.0", &home_url, &latest_url)
            .await
            .unwrap();

        assert!(result.has_update);
        assert_eq!(result.current_version, "v1.1.0");
        assert_eq!(result.latest_version, "v1.3.0");
        assert_eq!(result.release_notes, "manifest notes");
        assert_eq!(result.release_url, home_url);
        assert_eq!(
            result.download_url.as_deref(),
            Some("https://example.com/dit-pro.dmg")
        );
        assert_eq!(result.published_at, "2026-03-10T00:00:00Z");

        let seen = request_log.lock().unwrap().clone();
        assert!(seen.iter().any(|path| path == "/software/latest.json"));
        assert!(!seen.iter().any(|path| path == "/"));

        server.abort();
    }

    #[tokio::test]
    async fn test_check_for_update_falls_back_to_website_meta() {
        let mut routes = HashMap::new();
        routes.insert(
            "/".to_string(),
            TestResponse {
                status: "200 OK",
                content_type: "text/html",
                body: r#"
                    <html><head>
                      <meta content="v1.4.0" name="ditpro-stable-version">
                    </head><body>DIT Pro</body></html>
                "#
                .to_string(),
            },
        );
        routes.insert(
            "/software/latest.json".to_string(),
            TestResponse {
                status: "500 Internal Server Error",
                content_type: "application/json",
                body: "{}".to_string(),
            },
        );

        let (base_url, request_log, server) = spawn_test_server(routes).await;
        let home_url = format!("{}/", base_url);
        let latest_url = format!("{}/software/latest.json", base_url);

        let result = check_for_update_from_sources("1.4.0-beta.7", &home_url, &latest_url)
            .await
            .unwrap();

        assert!(result.has_update);
        assert_eq!(result.current_version, "v1.4.0-beta.7");
        assert_eq!(result.latest_version, "v1.4.0");
        assert!(result.release_notes.is_empty());
        assert_eq!(result.release_url, home_url);
        assert_eq!(result.download_url, None);

        let seen = request_log.lock().unwrap().clone();
        assert!(seen.iter().any(|path| path == "/software/latest.json"));
        assert!(seen.iter().any(|path| path == "/"));

        server.abort();
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("1.2.0", "1.3.0"));
        assert!(is_newer("1.2.0", "2.0.0"));
        assert!(is_newer("1.2.0", "1.2.1"));
        assert!(!is_newer("1.3.0", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2.0"));
        assert!(is_newer("v1.2.0", "v1.3.0")); // with v prefix
        assert!(is_newer("1.4.0-beta.7", "1.4.0"));
        assert!(!is_newer("1.4.0", "1.4.0-beta.7"));
    }
}
