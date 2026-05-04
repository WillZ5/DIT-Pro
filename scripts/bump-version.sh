#!/usr/bin/env bash
#
# bump-version.sh - Synchronize base version across Cargo.toml, package.json, package-lock.json, tauri.conf.json
#
# Usage:
#   ./scripts/bump-version.sh 0.6.0
#   ./scripts/bump-version.sh 1.0.0
#
# This script writes only MAJOR.MINOR.PATCH base versions. Pre-release suffixes
# are supplied at build time with DIT_PRE_RELEASE, for example beta.2 or rc.1.
# It does NOT commit or tag. Do that manually after review.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <new-version>"
    echo "  Example: $0 0.6.0"
    echo "  Example: $0 1.0.0"
    echo "  Pre-release example: DIT_PRE_RELEASE=beta.2 cargo tauri build"
    exit 1
fi

NEW_VERSION="$1"

# Validate SemVer format (basic check)
if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Error: Version must be in MAJOR.MINOR.PATCH format (e.g., 0.6.0)"
    exit 1
fi

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "Bumping version to $NEW_VERSION in:"

# 1. Cargo.toml
CARGO_FILE="$PROJECT_ROOT/src-tauri/Cargo.toml"
if [[ -f "$CARGO_FILE" ]]; then
    # Update the first version = "x.y.z" in [package] section
    sed -i '' -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?\"/version = \"$NEW_VERSION\"/" "$CARGO_FILE"
    echo "  [ok] $CARGO_FILE"
else
    echo "  [skip] $CARGO_FILE (not found)"
fi

# 2. package.json
PKG_FILE="$PROJECT_ROOT/package.json"
if [[ -f "$PKG_FILE" ]]; then
    # Use node for safe JSON editing, fall back to sed
    if command -v node &>/dev/null; then
        node -e "
            const fs = require('fs');
            const pkg = JSON.parse(fs.readFileSync('$PKG_FILE', 'utf8'));
            pkg.version = '$NEW_VERSION';
            fs.writeFileSync('$PKG_FILE', JSON.stringify(pkg, null, 2) + '\n');
        "
    else
        sed -i '' -E "s/\"version\": \"[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?\"/\"version\": \"$NEW_VERSION\"/" "$PKG_FILE"
    fi
    echo "  [ok] $PKG_FILE"
else
    echo "  [skip] $PKG_FILE (not found)"
fi

# 3. package-lock.json
LOCK_FILE="$PROJECT_ROOT/package-lock.json"
if [[ -f "$LOCK_FILE" ]]; then
    if command -v node &>/dev/null; then
        node -e "
            const fs = require('fs');
            const lock = JSON.parse(fs.readFileSync('$LOCK_FILE', 'utf8'));
            lock.version = '$NEW_VERSION';
            if (lock.packages && lock.packages['']) {
                lock.packages[''].version = '$NEW_VERSION';
            }
            fs.writeFileSync('$LOCK_FILE', JSON.stringify(lock, null, 2) + '\n');
        "
    else
        echo "  [skip] $LOCK_FILE (node required for safe JSON edit)"
        LOCK_FILE=""
    fi
    if [[ -n "$LOCK_FILE" ]]; then
        echo "  [ok] $LOCK_FILE"
    fi
else
    echo "  [skip] $LOCK_FILE (not found)"
fi

# 4. tauri.conf.json
TAURI_FILE="$PROJECT_ROOT/src-tauri/tauri.conf.json"
if [[ -f "$TAURI_FILE" ]]; then
    if command -v node &>/dev/null; then
        node -e "
            const fs = require('fs');
            const conf = JSON.parse(fs.readFileSync('$TAURI_FILE', 'utf8'));
            conf.version = '$NEW_VERSION';
            fs.writeFileSync('$TAURI_FILE', JSON.stringify(conf, null, 2) + '\n');
        "
    else
        sed -i '' -E "s/\"version\": \"[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?\"/\"version\": \"$NEW_VERSION\"/" "$TAURI_FILE"
    fi
    echo "  [ok] $TAURI_FILE"
else
    echo "  [skip] $TAURI_FILE (not found)"
fi

echo ""
echo "Done! Version bumped to $NEW_VERSION"
echo ""
echo "Next steps:"
echo "  1. Review changes: git diff"
echo "  2. Commit:         git commit -am 'chore: bump version to $NEW_VERSION'"
echo "  3. Stable tag:     git tag v$NEW_VERSION"
echo "  4. Beta tag:       git tag v$NEW_VERSION-beta.2"
echo "  5. Beta build:     DIT_PRE_RELEASE=beta.2 cargo tauri build"
echo "  6. Stable build:   DIT_PRE_RELEASE= cargo tauri build"
