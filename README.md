# DIT Pro

[English](README.md) | [中文](README_CN.md)

Professional card offload engine for film production. Built with Tauri 2.0, React/TypeScript, and Rust.

## Features

- **Multi-destination copy** with parallel write and 4MB atomic buffering
- **Hash verification** (XXH64 / XXH3 / XXH128 / SHA-256 / MD5), 10+ GB/s throughput
- **ASC MHL v2.0** manifest generation and chain integrity
- **Cascading copy** -- fast SSD first, then auto-cascade to slower targets
- **Conflict detection** with per-file resolution (skip / overwrite / keep both)
- **Checkpoint & resume** -- survives crashes, cable pulls, and power loss
- **IO scheduler** -- per-device concurrency control (HDD 1-2 / SSD 4 / NVMe 8)
- **Real-time progress** -- speed chart, ETA, per-file status
- **Shooting day reports** -- HTML/TXT export with file-level hash records
- **Email notifications** -- SMTP-based alerts on completion or error
- **Workflow presets** -- save and reuse offload configurations
- **Structured error codes** -- 33 codes across 8 categories with 4 severity levels
- **Debug bundle export** -- one-click diagnostics (logs, DB, config, system info)
- **i18n** -- English and Chinese

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Shell | [Tauri 2.0](https://v2.tauri.app/) |
| Frontend | React + TypeScript |
| Backend | Rust |
| Database | SQLite (WAL mode) |
| CLI | `mhl-verify` -- standalone MHL chain verifier |

## Requirements

- macOS 11.0+ (Apple Silicon native)
- [Node.js](https://nodejs.org/) 18+
- [Rust](https://www.rust-lang.org/tools/install) 1.77.2+

## Development

```bash
# Install dependencies
npm install

# Start dev server (frontend HMR + Tauri backend)
npm run dev

# Run Rust tests
cargo test --manifest-path src-tauri/Cargo.toml

# Build production DMG
npx @tauri-apps/cli build
```

## Documentation

- Architecture: [English](docs/ARCHITECTURE.md) | [中文](docs/ARCHITECTURE_CN.md)
- Changelog: [English](docs/CHANGELOG.md) | [中文](docs/CHANGELOG_CN.md)

## Project Structure

```
src/                        # React frontend
  views/                    # 5 main views: Jobs, Volumes, Presets, Reports, Settings
  i18n/                     # Internationalization (en/zh)
src-tauri/                  # Rust backend
  src/
    copy_engine/            # Multi-path parallel copy with atomic writes
      atomic_writer.rs      # Atomic .tmp -> final rename with Drop cleanup
    hash_engine/            # XXH64 / XXH3 / XXH128 / SHA-256 / MD5
    workflow/               # Offload orchestrator (scan -> copy -> verify -> MHL -> notify)
    mhl/                    # ASC MHL v2.0 manifest generation
    checkpoint/             # Crash recovery and resume
    volume/                 # Device discovery and space monitoring
    io_scheduler/           # Per-device concurrency control
    db/                     # SQLite schema and migrations
    config.rs               # JSON config persistence
    preset.rs               # Workflow preset management
    report.rs               # Shooting day and job reports (HTML/TXT)
    notify/                 # Email notification system (SMTP)
    tray.rs                 # System tray (idle/active/error states)
    error.rs                # Structured error code system (E1001-E1704)
    error_log.rs            # Error log persistence
    debug_bundle.rs         # One-click diagnostic export
    version.rs              # Semantic versioning and release channels
    commands.rs             # 44 Tauri IPC commands
mhl-verify-cli/             # Standalone CLI for MHL chain verification
```

## License

MIT License. See [LICENSE](LICENSE) for details.
