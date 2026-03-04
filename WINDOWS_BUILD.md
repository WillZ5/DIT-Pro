# DIT Pro — Windows Build Guide

This document provides step-by-step instructions for building DIT Pro on Windows.
Target: another Claude Code instance on a Windows machine should be able to follow these steps to compile and test.

---

## ⚠️ Important Rule

**Windows 机器只负责编译和上传产物，不要修改任何代码。**

如果编译失败，请将完整的错误信息反馈给用户，由 macOS 端修复后重新 push。Windows 端再 git pull 重试。

**禁止**：修改源码、创建 PR、修改 package.json/Cargo.toml 等任何文件。

---

## Prerequisites

### 1. Visual Studio Build Tools (MSVC)

Tauri/Rust on Windows requires the MSVC C++ toolchain. Install **one** of:

- **Visual Studio 2022 Community** (free): https://visualstudio.microsoft.com/
- **Visual Studio Build Tools 2022** (smaller, CLI-only): https://aka.ms/vs/17/release/vs_BuildTools.exe

During installation, select the **"Desktop development with C++"** workload. This includes:
- MSVC v143 C++ build tools
- Windows 10/11 SDK
- C++ CMake tools (optional but helpful)

### 2. Rust

Install via rustup: https://rustup.rs/

```powershell
# In PowerShell (Admin or normal)
winget install Rustlang.Rustup
# Or download and run rustup-init.exe from https://rustup.rs/

# Verify
rustup --version
rustc --version
cargo --version
```

Ensure the `stable-x86_64-pc-windows-msvc` toolchain is installed (default on Windows).

### 3. Node.js 20 LTS

```powershell
winget install OpenJS.NodeJS.LTS
# Or download from https://nodejs.org/

# Verify
node --version   # Should be v20.x or v22.x
npm --version
```

### 4. Git for Windows

```powershell
winget install Git.Git
# Or download from https://git-scm.com/download/win

# Verify
git --version
```

### 5. Tauri CLI

The project uses Tauri v2. The CLI is installed as a cargo subcommand:

```powershell
cargo install tauri-cli --version "^2"
# Verify
cargo tauri --version
```

---

## Clone & Setup

```powershell
# Clone the repository
git clone https://github.com/WillZ5/DIT-Pro.git
cd DIT-Pro

# Install frontend dependencies
npm ci
```

---

## Build Commands

### Development Build (with hot-reload)

```powershell
cargo tauri dev
```

This starts the Vite dev server on `http://localhost:5173` and opens the Tauri window.

### Release Build (NSIS Installer)

```powershell
cargo tauri build
```

**Build outputs** (after successful build):

| Output | Path |
|--------|------|
| NSIS installer (.exe) | `src-tauri\target\release\bundle\nsis\DIT Pro_1.1.0_x64-setup.exe` |
| Standalone executable | `src-tauri\target\release\dit-pro.exe` |

The NSIS installer is the distributable artifact for end users.

### Rust-only Build (no frontend, faster iteration)

```powershell
cd src-tauri
cargo build
```

### Run Tests

```powershell
cd src-tauri

# All unit tests
cargo test --lib --verbose

# Integration tests
cargo test --tests --verbose

# Doc tests
cargo test --doc --verbose

# Clippy lints
cargo clippy --all-targets -- -D warnings

# Format check
cargo fmt --all --check
```

---

## Expected Build Behavior

### First Build
- Takes 5-15 minutes depending on machine (downloads and compiles all crates)
- `windows` crate and `wmi` crate compile only on Windows targets
- Node.js `npm ci` installs frontend dependencies
- Vite builds the React frontend to `dist/`

### Windows-Specific Code Paths
The following modules have `#[cfg(windows)]` blocks that will compile on Windows:

| Module | Windows Implementation |
|--------|----------------------|
| `volume/mod.rs` | `GetDiskFreeSpaceExW`, `GetVolumeInformationW`, `GetDriveTypeW`, `GetLogicalDriveStringsW` + PowerShell WMI |
| `copy_engine/mod.rs` | Space check via `get_volume_space()` |
| `debug_bundle.rs` | `cmd /c ver`, `wmic`, `GlobalMemoryStatusEx` |
| `tray.rs` | Standard icon (no template mode) |
| `lib.rs` | Windows-style File/Help menu |
| `mhl/mod.rs` | `hostname` command (works on Windows) |
| `build.rs` | `chrono` for timestamp (cross-platform) |

### Volume Detection Testing
After building, manually test these scenarios:

1. **C: drive** should show as Internal SSD (or HDD depending on hardware)
2. **USB flash drive** should show as External/USB
3. **SD card via USB reader** should show as SD Card
4. **Mapped network drive** should show as Network
5. **External SSD** should show as External SSD

The volume detection uses a 3-tier approach:
- `GetDriveTypeW` for basic type (removable/fixed/network/cdrom)
- PowerShell `Get-Partition -DriveLetter X | Get-Disk | Select-Object MediaType,BusType` for SSD/HDD/USB detection
- Results are cached per-session for performance

---

## Troubleshooting

### "MSVC not found" or linker errors
- Ensure Visual Studio Build Tools are installed with "Desktop development with C++" workload
- Run `rustup show` to verify `stable-x86_64-pc-windows-msvc` is the active toolchain

### `npm ci` fails
- Delete `node_modules` folder and `package-lock.json`, then run `npm install`
- Ensure Node.js 20+ is installed

### WMI/PowerShell queries fail
- Volume detection gracefully falls back to `DeviceType::Unknown` if PowerShell fails
- Check that PowerShell execution policy allows scripts: `Get-ExecutionPolicy` (should be RemoteSigned or less restrictive)

### Long compile times
- First build is slow (5-15 min). Subsequent builds use cargo cache (~30s-2min for incremental)
- `windows` crate feature selection is already minimal (only 2 features enabled)

### Tauri build fails with signing error
- The `TAURI_SIGNING_PRIVATE_KEY` env var can be set to empty string `""` for unsigned builds
- Or just run `cargo tauri build` without it (CI sets it to empty)

---

## Architecture Notes

- **No cross-compilation**: Windows builds must run on Windows (MSVC toolchain required)
- **macOS code is untouched**: All `#[cfg(target_os = "macos")]` blocks remain unchanged
- **Path handling**: MHL paths are normalized to POSIX `/` via `.replace('\\', "/")` in workflow/mod.rs
- **Installer**: NSIS format produces a standard `.exe` installer with `currentUser` install mode
- **Tray icon**: Uses full-color PNG on Windows (no template mode, that's macOS-only)

---

## CI/CD

GitHub Actions automatically builds on `windows-latest` runner for:
- `rust-tests` job (cargo test + clippy + fmt)
- `build-windows` job (NSIS installer, artifact uploaded)

Triggered on push to `main`/`develop` and PRs to `main`.
