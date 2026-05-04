# Release And Version Guide

[English](RELEASE_GUIDE.md) | [中文](RELEASE_GUIDE_CN.md)

This document defines versioning, GitHub Release copy, installer naming, and AI agent update checks.

## Version Rules

DIT Pro uses semantic versioning for published versions:

```text
MAJOR.MINOR.PATCH[-prerelease]
```

Examples:

- `1.4.0-beta.2`
- `1.5.0-rc.1`
- `1.5.0`

Config files store the base version only: `MAJOR.MINOR.PATCH`. Pre-release suffixes are supplied at build time with `DIT_PRE_RELEASE`.

Example:

```text
Base config version: 1.4.0
Beta build env: DIT_PRE_RELEASE=beta.2
Displayed/release version: 1.4.0-beta.2
```

Before publishing or producing an external installer, update all base version files:

- `package.json`
- `package-lock.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`

Do not write `1.4.0-beta.2` into those config files. That suffix belongs in `DIT_PRE_RELEASE` and the public release tag/title.

Decision rules:

- `docs-only`: no version bump.
- `internal-only`: no version bump unless it is included in an external installer.
- `bugfix`: patch bump when shipped externally.
- `user-visible feature`: minor bump when shipped externally; beta/RC builds may use a prerelease suffix.
- `distribution`: any DMG/EXE, GitHub Release, or `latest*.json` update must confirm all base versions match and the pre-release suffix matches the public tag.

## GitHub Release Format

### Title

Use this format:

```text
vX.Y.Z[-prerelease] - Short release title
```

Correct:

```text
v1.5.0-beta.2 - Cloud Upload Queue and Proxy Delivery
```

Do not use:

```text
v1.5.0-beta.2 [em dash] Cloud Upload Queue and Proxy Delivery
v1.5.0-beta.2 [en dash] Cloud Upload Queue and Proxy Delivery
v1.5.0-beta.2 - [emoji] Cloud Upload Queue
```

### Body

Use this template:

```markdown
## Summary

One or two short paragraphs describing the release.

## Changes

- Added ...
- Fixed ...
- Changed ...

## Downloads

| Platform | File |
| --- | --- |
| macOS Universal | `DIT.Pro_X.Y.Z_universal.dmg` |
| Windows x64 | `DIT.Pro_X.Y.Z_x64-setup.exe` |

## Verification

- `cargo fmt --all --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `npm run build`
- `npm run test:e2e`

## Known Issues

- None.
```

Rules:

- Do not use emoji.
- Do not use em dash (U+2014) or en dash (U+2013); use ASCII `-`.
- Do not include internal roadmap details, unreleased plans, or sensitive customer information.
- GitHub Releases are public and should default to English. Chinese may be added after English without changing the structure.
- Verification must list commands that actually ran. Do not claim a command passed if it was not run.

## Installers And Website Downloads

GitHub Release asset names:

- `DIT.Pro_X.Y.Z_universal.dmg`
- `DIT.Pro_X.Y.Z_x64-setup.exe`

Vercel website fallback asset names:

- `stable-DIT.Pro_X.Y.Z_universal.dmg`
- `stable-DIT.Pro_X.Y.Z_x64-setup.exe`
- `beta-DIT.Pro_X.Y.Z_universal.dmg`
- `beta-DIT.Pro_X.Y.Z_x64-setup.exe`

`website/software/` is the mainland China download fallback directory, not cleanup junk. When updating installers, update:

- The matching DMG/EXE files.
- `website/software/latest.json` or `website/software/latest-beta.json`.
- GitHub Release asset URLs.
- Vercel fallback URLs.

## AI Agent Update Checklist

Every update handoff must check:

- Which milestone the work belongs to.
- Whether the change is user-visible.
- Whether the base version or `DIT_PRE_RELEASE` must change.
- Whether the Changelog or release notes must change.
- Whether installers, auto-update metadata, website downloads, or `website/software/` are affected.
- Whether internal Feishu docs need syncing.

Final summaries must state:

- Whether the base version or pre-release suffix changed. If not, why not.
- Milestone impact.
- Verification commands that were run.
- For releases, whether the Release title follows `vX.Y.Z - Title`.
