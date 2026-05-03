# Milestone Development Process

[English](MILESTONES.md) | [中文](MILESTONES_CN.md)

This document defines how DIT Pro milestones are planned. The roadmap describes direction; milestones define the scope, acceptance criteria, and release gates for a development cycle.

## Milestone Naming

Use this format:

```text
vX.Y.Z[-prerelease] - Theme
```

Example:

```text
v1.5.0-beta - Cloud Upload Queue and Proxy Delivery
```

Rules:

- Use ASCII hyphen-minus: ` - `.
- Do not use em dash (U+2014) or en dash (U+2013).
- Do not use emoji.
- Describe the deliverable theme, not an internal codename.

## Milestone States

- `Proposed`: suggested but not confirmed.
- `Planned`: scope, priority, and acceptance criteria are confirmed.
- `In Progress`: implementation is active.
- `Code Complete`: implementation is complete and validation has started.
- `Release Candidate`: scope is frozen; only blocking fixes are accepted.
- `Shipped`: released and documented.
- `Deferred`: moved or split into a later milestone.

## Required Milestone Fields

- `Goal`: one sentence describing the problem this cycle solves.
- `Scope`: features and fixes explicitly included in this cycle.
- `Out of scope`: work that should not be pulled into this cycle.
- `Priority`: at least P0/P1/P2.
- `Acceptance tests`: user behaviors and engineering checks that prove completion.
- `Version impact`: expected version type: patch, minor, major, beta, or RC.
- `Distribution impact`: whether DMG/EXE, GitHub Release, or Vercel fallback installers are affected.
- `Docs impact`: whether README, Roadmap, Changelog, Release notes, or Feishu need updates.

## Development Gates

### 1. Planning Gate

Before implementation, confirm:

- Which milestone the work belongs to.
- Whether the change is user-visible.
- Whether a version bump is required.
- Whether installers, website downloads, auto-update metadata, or release notes are affected.

### 2. Implementation Gate

During implementation:

- Do not introduce large refactors unrelated to the milestone.
- Do not split independent tools into separate products unless the milestone explicitly requires it.
- Do not delete installer fallback files in `website/software/`.

### 3. Validation Gate

Before merge or release:

- All P0 acceptance items pass.
- Versions match in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`.
- If external packages are produced, installers, manifests, and release notes agree.
- GitHub Release copy follows `docs/RELEASE_GUIDE.md`.

### 4. Release Gate

Before publishing:

- Git tag: `vX.Y.Z[-prerelease]`.
- GitHub Release title: `vX.Y.Z[-prerelease] - Short title`.
- No emoji and no em dash.
- Release notes include Summary, Changes, Downloads, Verification, and Known Issues.
- Mainland China download fallback is verified: `website/software/` files match `latest*.json`.

## Current Development Direction

The current milestone should organize these P0 areas:

- Stabilize in-app MHL verification.
- Cloud upload queue and slow-network storage workflow.
- Proxy-only review delivery.
- Integrity verification for Baidu Netdisk, Google Drive, and WebDAV/AList.

Use `docs/ROADMAP.md` and the active GitHub milestone for the exact scope.
