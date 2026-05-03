## Summary


## Milestone

- Milestone:
- Scope:
- Out of scope:

## Version And Release Impact

- Change type: docs-only / internal-only / bugfix / user-visible feature / release / distribution
- Version bump required: yes / no
- Version files updated if required:
  - `package.json`
  - `src-tauri/Cargo.toml`
  - `src-tauri/tauri.conf.json`
- Changelog or release notes updated: yes / no / not needed
- GitHub Release format checked: yes / no / not needed

## Distribution Impact

- DMG/EXE changed: yes / no
- `website/software/` fallback changed: yes / no
- `latest.json` or `latest-beta.json` changed: yes / no

## Verification

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `npm run build`
- [ ] `npm run test:e2e`
- [ ] Other:

## Notes

- No emoji in public GitHub copy.
- GitHub Release titles use ASCII hyphen: `vX.Y.Z - Title`.
- Do not delete `website/software/` installers during cleanup.
