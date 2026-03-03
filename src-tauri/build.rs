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
    if let Ok(output) = std::process::Command::new("date")
        .args(["-u", "+%Y%m%d"])
        .output()
    {
        if output.status.success() {
            let date = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=DIT_BUILD_TIME={}", date);
        }
    }

    // ── Pre-release tag ────────────────────────────────────────────
    // Override via env: DIT_PRE_RELEASE=rc.1 cargo tauri build
    // Set to empty string for stable release: DIT_PRE_RELEASE= cargo tauri build
    let pre = std::env::var("DIT_PRE_RELEASE").unwrap_or_else(|_| String::new()); // default: stable
    if !pre.is_empty() {
        println!("cargo:rustc-env=DIT_PRE_RELEASE={}", pre);
    }

    // ── Rerun triggers ────────────────────────────────────────────
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-env-changed=DIT_PRE_RELEASE");

    // ── Tauri build ───────────────────────────────────────────────
    tauri_build::build()
}
