# 发布与版本规范

[English](RELEASE_GUIDE.md) | [中文](RELEASE_GUIDE_CN.md)

本文档定义版本号、GitHub Release 文案、安装包命名和 AI agent 更新检查规则。

## 版本号规则

DIT Pro 使用语义化版本：

```text
MAJOR.MINOR.PATCH[-prerelease]
```

示例：

- `1.4.0-beta`
- `1.5.0-rc.1`
- `1.5.0`

发布或生成对外安装包前，必须同步更新三处：

- `package.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`

判定规则：

- `docs-only`：不改版本号。
- `internal-only`：不改版本号，除非会进入对外安装包。
- `bugfix`：对外发布时至少 patch bump。
- `user-visible feature`：对外发布时 minor bump，beta/RC 可带 prerelease。
- `distribution`：只要更新 DMG/EXE、GitHub Release、`latest*.json`，必须确认三处版本一致。

## GitHub Release 格式

### Title

统一格式：

```text
vX.Y.Z[-prerelease] - Short release title
```

正确：

```text
v1.5.0-beta - Cloud Upload Queue and Proxy Delivery
```

禁止：

```text
v1.5.0-beta [em dash] Cloud Upload Queue and Proxy Delivery
v1.5.0-beta [en dash] Cloud Upload Queue and Proxy Delivery
v1.5.0-beta - [emoji] Cloud Upload Queue
```

### Body

统一模板：

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

规则：

- 不使用 emoji。
- 不使用 em dash（U+2014）或 en dash（U+2013），统一使用 ASCII `-`。
- 不写内部规划、未发布路线图或敏感客户信息。
- GitHub Release 面向外部用户，语言默认英文；如需中文，放在英文之后，不改变结构。
- 测试项必须写实际跑过的命令；没跑就不要写通过。

## 安装包与官网下载

GitHub Release 资源命名：

- `DIT.Pro_X.Y.Z_universal.dmg`
- `DIT.Pro_X.Y.Z_x64-setup.exe`

Vercel 官网兜底资源命名：

- `stable-DIT.Pro_X.Y.Z_universal.dmg`
- `stable-DIT.Pro_X.Y.Z_x64-setup.exe`
- `beta-DIT.Pro_X.Y.Z_universal.dmg`
- `beta-DIT.Pro_X.Y.Z_x64-setup.exe`

`website/software/` 是中国大陆下载兜底目录，不是垃圾目录。更新安装包时必须同步：

- 对应 DMG/EXE 文件。
- `website/software/latest.json` 或 `website/software/latest-beta.json`。
- GitHub Release asset URL。
- Vercel fallback URL。

## AI Agent 更新检查清单

每次接手更新时必须检查：

- 当前工作属于哪个 milestone。
- 是否用户可见。
- 是否要修改版本号。
- 是否要更新 Changelog 或 release note。
- 是否影响安装包、自动更新、官网下载或 `website/software/`。
- 是否需要同步飞书内部文档。

完成时必须汇报：

- 版本号是否修改；未修改则说明原因。
- milestone 影响。
- 运行过的验证命令。
- 如涉及发布，Release title 是否符合 `vX.Y.Z - Title` 格式。
