# Architecture

[English](ARCHITECTURE.md) | [中文](ARCHITECTURE_CN.md)

## Overview

DIT Pro is a Tauri 2.0 desktop application with a React/TypeScript frontend and a Rust backend. It targets macOS (Apple Silicon native) for professional film production card offloading.

```
┌──────────────────────────────────────┐
│           macOS Desktop              │
│  ┌────────────────────────────────┐  │
│  │     React + TypeScript UI      │  │
│  │   (Jobs, Volumes, Presets,     │  │
│  │    Reports, Rushes Log,        │  │
│  │    MHL Verify, Settings)       │  │
│  └──────────┬─────────────────────┘  │
│             │ Tauri IPC commands     │
│  ┌──────────▼─────────────────────┐  │
│  │        Rust Backend            │  │
│  │  ┌─────────┐ ┌──────────────┐  │  │
│  │  │ Copy    │ │ Hash Engine  │  │  │
│  │  │ Engine  │ │ XXH64/XXH3/  │  │  │
│  │  │         │ │ XXH128/SHA/MD│  │  │
│  │  └────┬────┘ └──────┬───────┘  │  │
│  │       │             │          │  │
│  │  ┌────▼─────────────▼───────┐  │  │
│  │  │   Workflow Orchestrator  │  │  │
│  │  │ scan->copy->verify->MHL  │  │  │
│  │  └──────────┬───────────────┘  │  │
│  │             │                  │  │
│  │  ┌──────────▼───────────────┐  │  │
│  │  │   SQLite (WAL mode)      │  │  │
│  │  │   Checkpoint + ErrorLog  │  │  │
│  │  └──────────────────────────┘  │  │
│  └────────────────────────────────┘  │
└──────────────────────────────────────┘
```

## Rust Modules

| Module | Type | Description |
|--------|------|-------------|
| `copy_engine/` | directory | Multi-path parallel copy with 4MB atomic buffering |
| `copy_engine/atomic_writer.rs` | file | Atomic .tmp write + rename, auto-cleanup via `Drop` |
| `hash_engine/` | directory | XXH64, XXH3, XXH128, SHA-256, MD5 with `MultiHasher` |
| `workflow/` | directory | Offload orchestrator: scan -> copy -> verify -> MHL -> notify |
| `mhl/` | directory | ASC MHL v2.0 XML generation, chain management, and chain verification |
| `checkpoint/` | directory | Crash recovery, task state persistence, resume logic |
| `volume/` | directory | Device discovery, type detection (HDD/SSD/NVMe/RAID), space monitoring |
| `io_scheduler/` | directory | Per-device concurrency control and buffer sizing |
| `db/` | directory | SQLite schema, migrations, WAL mode initialization |
| `notify/` | directory | SMTP email notifications on completion/error |
| `config.rs` | file | JSON config read/write |
| `preset.rs` | file | Workflow preset CRUD |
| `report.rs` | file | Shooting day + job reports, HTML/TXT rendering |
| `tray.rs` | file | System tray with idle/active/error states |
| `error.rs` | file | 33 structured error codes (E1001-E1704), 8 categories |
| `error_log.rs` | file | Error log persistence to SQLite |
| `debug_bundle.rs` | file | One-click diagnostic ZIP export |
| `version.rs` | file | Semantic versioning, 5 release channels, build metadata |
| `commands.rs` | file | Tauri IPC command handlers |

## Offload Pipeline

```
1. Scan Source       Enumerate files, calculate total size
        │
2. Detect Conflicts  Compare against destinations, hash same-size files
        │
3. User Resolution   Skip / Overwrite / Keep Both (per-file)
        │
4. Copy Files        Parallel write to all destinations
        │                Atomic .tmp -> rename
        │                Per-device IO scheduling
        │                Checkpoint after each file
        │
5. Verify Hashes     Re-hash destination files, compare with source
        │
6. Generate MHL      ASC MHL v2.0 manifest for each destination
        │
7. Notify            Email report + update system tray
```

## Error Code System

| Range | Category | Description |
|-------|----------|-------------|
| E1000-E1099 | COPY | File copy operations |
| E1100-E1199 | HASH | Hash computation and verification |
| E1200-E1299 | DB | Database operations |
| E1300-E1399 | CONFIG | Configuration management |
| E1400-E1499 | IO | Storage and filesystem I/O |
| E1500-E1599 | MHL | ASC MHL chain operations |
| E1600-E1699 | NETWORK | Email and network |
| E1700-E1799 | SYSTEM | Runtime and system errors |

Severity levels: **Critical** (data loss risk) > **Error** (operation failed) > **Warning** (completed with issues) > **Info** (notification)

## Version Management

Version format: `MAJOR.MINOR.PATCH[-prerelease][+buildmeta]`

| Channel | Condition | Purpose |
|---------|-----------|---------|
| Dev | Version `0.x`, no pre-release tag | Local development |
| Alpha | Pre-release starts with `alpha` | Internal testing |
| Beta | Pre-release starts with `beta` | Extended testing |
| RC | Pre-release starts with `rc` | Release candidate |
| Stable | Version `1.0.0+`, no pre-release | Production release |

Build metadata is injected at compile time via `build.rs`:
- Git commit hash (7 chars)
- Build date (YYYYMMDD UTC)
- Pre-release tag (via `DIT_PRE_RELEASE` env var)
