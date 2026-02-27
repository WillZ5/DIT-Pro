# Changelog

[English](CHANGELOG.md) | [中文](CHANGELOG_CN.md)

All notable changes to DIT Pro are documented here.

Version format: [Semantic Versioning](https://semver.org/) `MAJOR.MINOR.PATCH[-prerelease]`

## Version History

| Version | Milestone | Status |
|---------|-----------|--------|
| v0.1.0 | Core engine | Done |
| v0.2.0 | Pipeline integration | Done |
| v0.3.0 | Advanced features | Done |
| v0.4.0 | UI/UX + i18n + bug fixes | Done |
| v0.5.0 | Error system + version management | Done |
| v1.0.0-alpha.1 | Production hardening (CSP, logging, signing) | Done |
| v1.0.0-alpha.2 | Bug fixes + conflict detection | Done |
| v1.0.0-beta.1 | AC verification tests + frontend polish | Done |
| v1.0.0-beta.2 | State management overhaul + backend hardening | Done |
| v1.0.0-beta.3 | Serialization fix + speed chart + rename to DIT Pro | **Current** |

---

## v1.0.0-beta.3 (2026-02-27)

**Tests**: 117 passing | **Channel**: Beta

### Bug Fixes
- **Speed chart not displaying (root cause)**: Serde `rename_all = "camelCase"` on enums only renames variant names, not field names. Added `rename_all_fields = "camelCase"` (serde 1.0.185+)
- **ETA jumping erratically**: Changed from instantaneous speed to overall average speed (`completedBytes / elapsedSecs`)
- **Speed chart line thickness inconsistent**: Added `vectorEffect="non-scaling-stroke"` + moving average smoothing (window=5)

### Improvements
- Speed chart moved into expandable detail panel
- 1-second sampling interval (was ~100ms per event)
- Removed grid lines and Y-axis labels for cleaner visual
- History extended from 60 to 120 data points (2 minutes)
- Removed all debug logging

### Rename
- **DIT System -> DIT Pro** across 20+ files (61 occurrences)
- Removed all "Bulletproof" references
- New bundle identifier: `com.ditpro.desktop`

---

## v1.0.0-beta.2 (2026-02-26)

**Tests**: 139 passing | **Channel**: Beta

### Frontend State Management Overhaul
- Fixed task name disappearing after offload start
- Eliminated status flickering during pause/resume
- Fixed PhaseChanged event abuse (now only fires on real phase transitions)
- Fixed event race conditions with placeholder entry creation
- Fixed speed spike after resume (reset speed baseline)
- Added `useMemo` optimization for active/DB job lists

### Backend Hardening
- Verification phase now pausable/cancellable
- AtomicWriter auto-cleanup via `Drop` trait (no more orphaned .tmp files)

---

## v1.0.0-beta.1 (2026-02-26)

**Tests**: 128 passing | **Channel**: Beta

### AC Verification Tests
- 13 acceptance tests: AC-3 (MHL), AC-5 (Reports), AC-7 (CLI)
- Frontend i18n fixes and success banner component

---

## v1.0.0-alpha.2 (2026-02-26)

**Channel**: Alpha

### New Features
- **Conflict detection**: Pre-offload scan with per-file resolution dialog (skip / overwrite / keep both)
- Hash-based duplicate detection for same-size files

### Bug Fixes
- Speed chart initial display issues
- Crash recovery: orphan jobs now show resume button correctly

---

## v1.0.0-alpha.1 (2026-02-26)

**Channel**: Alpha

### Production Hardening
- Release logging enabled (Warn level)
- Content Security Policy configured
- macOS entitlements (minimal permissions)
- DMG-only packaging, minimum macOS 11.0

---

## v0.5.0 (2026-02-26)

### New Features
- Structured error code system: 33 codes (E1001-E1704), 8 categories, 4 severity levels
- Error log persistence (SQLite)
- Debug bundle export (logs + DB + config + system info)
- Version management with build metadata injection
- 5 release channels: Dev -> Alpha -> Beta -> RC -> Stable

---

## v0.4.0 (2026-02-25)

### New Features
- i18n: English + Chinese with auto-detection
- Browser preview mode with mock data
- ErrorBoundary component
- System tray with 3-state icons
- Resume offload command

### UI/UX
- SVG icons replacing emoji
- Dark theme (Zinc/Slate)
- Status badge capsules

---

## v0.3.0 (2026-02-24)

### New Features
- Cascading copy
- Workflow presets
- Report generation (HTML/TXT)
- Email notification system (SMTP)
- `mhl-verify-cli` standalone tool

---

## v0.2.0 (2026-02-23)

### New Features
- 25+ Tauri IPC commands
- Offload workflow orchestrator
- Real-time event system
- Settings persistence
- 5-view frontend framework

---

## v0.1.0 (2026-02-22)

### Initial Release
- Copy engine with multi-path parallel write
- Hash engine: XXH64/XXH3/SHA-256/MD5 (10+ GB/s)
- Checkpoint system (SQLite WAL)
- ASC MHL v2.0 support
- IO scheduler with per-device concurrency
- Volume discovery and space monitoring
