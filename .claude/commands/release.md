# Release Workflow - DIT Pro

Execute a full release workflow for DIT Pro. Ask the user for the new version number if not provided as `$ARGUMENTS`.

## Steps

### 0. Read Project Rules
Read these files before changing release state:
- `AGENTS.md`
- `docs/MILESTONES.md`
- `docs/RELEASE_GUIDE.md`
- `website/software/README.md`

### 1. Determine Version
- If `$ARGUMENTS` contains a version (e.g. `1.2.0`), use it.
- Otherwise, read the current version from `package.json` and ask the user what the new version should be (patch / minor / major bump, or a specific version).

### 2. Collect Release Notes
Ask the user for:
- **里程碑描述**（Milestone）：一句话概括本次发布内容
- **主要变更列表**：新功能、修复、优化等

If the user provides `$ARGUMENTS` with notes after the version, use those. Otherwise, check recent git log since last tag to help draft notes.

### 3. Version Bump
Update the version string in ALL THREE files (they must stay in sync):
- `package.json` → `"version": "X.Y.Z"`
- `src-tauri/Cargo.toml` → `version = "X.Y.Z"` under `[package]`
- `src-tauri/tauri.conf.json` → `"version": "X.Y.Z"`

### 4. Pre-flight Checks
Run these in parallel:
- **Rust formatting**: `cargo fmt --all --check` in `src-tauri/`
- **Clippy**: `cargo clippy --all-targets -- -D warnings` in `src-tauri/`
- **TypeScript type check**: `npx tsc --noEmit`

If any fail, fix the issues before continuing.

### 5. Run Tests
- `cargo test --lib --verbose` in `src-tauri/`
- `cargo test --tests --verbose` in `src-tauri/`

If tests fail, stop and report the failure. Do NOT proceed.

### 6. Build Verification
Run a release build to verify it compiles:
```
cargo tauri build --target universal-apple-darwin
```
If this fails, stop and report.

### 7. Git Commit & Tag
- Stage the 3 version-bumped files
- Commit with message: `release: v{VERSION}`
- Create an annotated git tag: `git tag -a v{VERSION} -m "Release v{VERSION}"`

### 8. Prepare GitHub Release Notes
Use `docs/RELEASE_GUIDE.md`.

Required title format:

```
v{VERSION} - Short release title
```

Rules:
- Use ASCII hyphen-minus (`-`), not em dash (U+2014) or en dash (U+2013).
- Do not use emoji.
- Include Summary, Changes, Downloads, Verification, and Known Issues.
- Do not claim any verification command passed unless it actually ran.

### 9. Update Feishu Changelog
Update the Feishu page「版本发布记录」using the current project wiki references in `CLAUDE.md`.

Add a new version section at the top of the detailed changelog. Follow this format:

```
## v{VERSION}（{TODAY's DATE in YYYY-MM-DD}）
**Tests**: {test count} 通过 | **Channel**: {Stable/RC/Beta/Alpha}
### {category heading}
1. **{feature/fix name}**：{description}
...
### 构建
- **GitHub Release**: v{VERSION}
- **DMG**: DIT Pro_{VERSION}_universal.dmg (Universal: Intel x86_64 + Apple Silicon arm64)
- **测试**: {test count} tests 全通过
- **质量门禁**: cargo fmt / clippy -D warnings / npm build 全部零错误零警告
```

Use the release notes collected in Step 2 to populate the content. Match the language style and formatting of existing entries (Chinese descriptions, bold key terms, numbered lists).

### 10. Website Download Fallbacks
If installers changed, update `website/software/` according to `website/software/README.md`:
- Stable release: update `latest.json`.
- Beta/RC release: update `latest-beta.json`.
- Keep GitHub Release source URLs and Vercel fallback URLs aligned.

### 11. Summary
Print a summary:
- Old version -> New version
- Files modified
- Git tag created
- Feishu changelog updated (with link to the page)
- GitHub Release title and format checked
- Website fallback installers updated or explicitly not needed
- Remind user to `git push && git push --tags` to trigger the CI release pipeline
- Do NOT push automatically - let the user decide when to push
