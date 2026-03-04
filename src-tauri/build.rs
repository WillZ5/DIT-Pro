fn main() {
    // ── Git metadata ──────────────────────────────────────────────
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
    {
        if output.status.success() {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=DIT_GIT_HASH={}", hash);
        }
    }

    // ── Build timestamp (YYYYMMDD UTC) ────────────────────────────
    // Uses chrono instead of `date` command for cross-platform support.
    let date = chrono::Utc::now().format("%Y%m%d").to_string();
    println!("cargo:rustc-env=DIT_BUILD_TIME={}", date);

    // ── Pre-release tag ────────────────────────────────────────────
    // Override via env: DIT_PRE_RELEASE=rc.1 cargo tauri build
    // Set to empty string for stable release: DIT_PRE_RELEASE= cargo tauri build
    let pre = std::env::var("DIT_PRE_RELEASE").unwrap_or_else(|_| "beta.1".to_string()); // default: beta.1 for dev
    if !pre.is_empty() {
        println!("cargo:rustc-env=DIT_PRE_RELEASE={}", pre);
    }

    // ── Rerun triggers ────────────────────────────────────────────
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-env-changed=DIT_PRE_RELEASE");

    // ── Tauri build ───────────────────────────────────────────────
    tauri_build::build()
}
