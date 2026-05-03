# DIT Pro Agent Rules

These rules apply to AI agents and human maintainers working in this repository.

## Before Any Update

- Check the current milestone in `docs/MILESTONES.md` or `docs/MILESTONES_CN.md`.
- Classify the change as one of: `docs-only`, `internal-only`, `bugfix`, `user-visible feature`, `release`, or `distribution`.
- Decide whether a version bump is required before editing files.
- Keep `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` versions identical whenever a release or external build is produced.
- Do not reintroduce a standalone `mhl-verify-cli`; MHL verification is an in-app DIT Pro feature.
- Do not treat `website/software/` as cleanup junk. It stores Vercel-served installer fallbacks for users who cannot reach GitHub Releases from mainland China.

## Version Bump Rules

- No version bump is required for docs-only changes, local cleanup, tests, or internal refactors that are not shipped.
- A patch bump is required for user-visible fixes that will be shipped.
- A minor bump is required for user-visible features, workflow additions, or new provider support.
- A prerelease suffix is required for beta/RC builds: `1.5.0-beta`, `1.5.0-rc.1`.
- Before producing a DMG, EXE, GitHub Release, or website download manifest, update all three version files.

## Release Format Rules

- Use ASCII hyphen-minus (`-`) for release titles, not em dash (U+2014) or en dash (U+2013).
- Do not use emoji in GitHub releases, tags, PR titles, commit messages, or public docs.
- GitHub Release title format: `vX.Y.Z[-prerelease] - Short release title`.
- Tag format: `vX.Y.Z[-prerelease]`.
- Public release notes must follow `docs/RELEASE_GUIDE.md`.

## Required Final Checks

- Confirm milestone impact and version decision in the final summary.
- List tests that were run. If tests were not run, state why.
- If distribution files changed, verify `website/software/latest*.json` and the Vercel fallback installers.
