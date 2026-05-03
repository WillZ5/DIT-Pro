# Milestone 开发流程

[English](MILESTONES.md) | [中文](MILESTONES_CN.md)

本文档定义 DIT Pro 的 milestone 规划方式。Roadmap 描述方向；milestone 用来约束当前开发周期的范围、验收和发布门禁。

## Milestone 命名

统一格式：

```text
vX.Y.Z[-prerelease] - 主题
```

示例：

```text
v1.5.0-beta - Cloud Upload Queue and Proxy Delivery
```

规则：

- 使用 ASCII hyphen-minus：` - `。
- 不使用 em dash（U+2014）或 en dash（U+2013）。
- 不使用 emoji。
- 标题要描述交付主题，不写内部代号。

## Milestone 状态

- `Proposed`：提出但未确认。
- `Planned`：范围、优先级、验收条件已确认。
- `In Progress`：正在实现。
- `Code Complete`：功能完成，进入验证。
- `Release Candidate`：已冻结范围，只接受阻塞级修复。
- `Shipped`：已发布并同步文档。
- `Deferred`：延期或拆分到后续 milestone。

## 每个 Milestone 必须包含

- `Goal`：一句话说明这个周期要解决什么问题。
- `Scope`：本周期明确要做的功能和修复。
- `Out of scope`：本周期不做的内容，避免范围漂移。
- `Priority`：至少分出 P0/P1/P2。
- `Acceptance tests`：可验证的用户行为和工程测试。
- `Version impact`：预计版本号类型：patch、minor、major、beta、RC。
- `Distribution impact`：是否需要 DMG/EXE、GitHub Release、Vercel fallback installer。
- `Docs impact`：是否需要更新 README、Roadmap、Changelog、Release notes、飞书。

## 开发门禁

### 1. Planning Gate

开始实现前必须确认：

- 当前工作属于哪个 milestone。
- 是否是用户可见变更。
- 是否需要版本号变更。
- 是否会影响安装包、网站下载、自动更新或发布说明。

### 2. Implementation Gate

实现期间必须保持：

- 不引入与 milestone 无关的大规模重构。
- 不把独立工具拆出为新产品，除非 milestone 明确要求。
- 不删除 `website/software/` 中的安装包兜底文件。

### 3. Validation Gate

合并或发布前至少确认：

- P0 验收项全部通过。
- 版本号三处一致：`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`。
- 如产生对外包，安装包、manifest、release notes 一致。
- GitHub Release 文案符合 `docs/RELEASE_GUIDE_CN.md`。

### 4. Release Gate

发布前必须完成：

- Git tag：`vX.Y.Z[-prerelease]`。
- GitHub Release title：`vX.Y.Z[-prerelease] - Short title`。
- 无 emoji，无 em dash。
- 发布说明包含 Summary、Changes、Downloads、Verification、Known Issues。
- 中国大陆下载兜底已验证：`website/software/` 文件和 `latest*.json` 对齐。

## 当前开发方向

当前 milestone 应围绕以下 P0 工作组织：

- 应用内 MHL 验证继续稳定化。
- 云上传队列与慢速网络存储流程。
- Proxy-only 快速审片交付。
- 百度网盘、Google Drive、WebDAV/AList 的完整性验证策略。

具体范围以 `docs/ROADMAP_CN.md` 和当期 GitHub milestone 为准。
