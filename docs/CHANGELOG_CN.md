# 更新日志

[English](CHANGELOG.md) | [中文](CHANGELOG_CN.md)

DIT Pro 的重要变更记录如下。

版本格式遵循 [Semantic Versioning](https://semver.org/)：`MAJOR.MINOR.PATCH[-prerelease]`

## 版本历史

| 版本 | 里程碑 | 状态 |
|------|--------|------|
| v0.1.0 | 核心引擎 | 已完成 |
| v0.2.0 | 流水线集成 | 已完成 |
| v0.3.0 | 高级功能 | 已完成 |
| v0.4.0 | UI/UX + i18n + Bug 修复 | 已完成 |
| v0.5.0 | 错误系统 + 版本管理 | 已完成 |
| v1.0.0-alpha.1 | 生产加固（CSP、日志、签名） | 已完成 |
| v1.0.0-alpha.2 | Bug 修复 + 冲突检测 | 已完成 |
| v1.0.0-beta.1 | AC 验收测试 + 前端打磨 | 已完成 |
| v1.0.0-beta.2 | 状态管理重构 + 后端加固 | 已完成 |
| v1.0.0-beta.3 | 序列化修复 + 速度图优化 + 更名 DIT Pro | **当前** |

---

## v1.0.0-beta.3（2026-02-27）

**测试**：117 通过 | **通道**：Beta

### Bug 修复
- **速度图不显示（根因）**：Serde 在 enum 上的 `rename_all = "camelCase"` 只会重命名变体名，不会重命名字段名。新增 `rename_all_fields = "camelCase"`（serde 1.0.185+）
- **ETA 抖动明显**：从瞬时速度改为整体平均速度（`completedBytes / elapsedSecs`）
- **线条粗细不一致**：增加 `vectorEffect="non-scaling-stroke"`，并加入滑动平均（窗口 5）

### 优化
- 速度图移动到可展开详情区域
- 采样频率改为 1 秒（此前约 100ms）
- 移除网格线与 Y 轴刻度标签，降低视觉噪声
- 历史长度从 60 增至 120 点（2 分钟）
- 移除全部 debug 日志

### 更名
- **DIT System -> DIT Pro**（20+ 文件，61 处）
- 移除所有 “Bulletproof” 文案
- 新包标识符：`com.ditpro.desktop`

---

## v1.0.0-beta.2（2026-02-26）

**测试**：139 通过 | **通道**：Beta

### 前端状态管理重构
- 修复任务启动后名称丢失
- 修复暂停/恢复时状态闪烁
- 修复 `PhaseChanged` 事件滥发（仅真正切换阶段时触发）
- 修复事件竞态（占位条目创建）
- 修复恢复后速度尖刺（重置速度基线）
- 为活跃任务与 DB 任务列表增加 `useMemo` 优化

### 后端加固
- 校验阶段支持暂停/取消
- `AtomicWriter` 通过 `Drop` 自动清理临时文件（避免孤儿 `.tmp`）

---

## v1.0.0-beta.1（2026-02-26）

**测试**：128 通过 | **通道**：Beta

### AC 验收测试
- 13 个验收测试：AC-3（MHL）、AC-5（报告）、AC-7（CLI）
- 前端 i18n 修复与 success banner 组件

---

## v1.0.0-alpha.2（2026-02-26）

**通道**：Alpha

### 新功能
- **冲突检测**：拷卡前扫描冲突，逐文件决策（跳过 / 覆盖 / 保留两者）
- 同大小文件支持哈希级重复判断

### Bug 修复
- 速度图初始显示问题
- 崩溃恢复后孤儿任务按钮显示问题

---

## v1.0.0-alpha.1（2026-02-26）

**通道**：Alpha

### 生产加固
- Release 构建启用日志（Warn 级）
- 配置 Content Security Policy
- macOS 最小权限 entitlements
- 仅打包 DMG，最低 macOS 11.0

---

## v0.5.0（2026-02-26）

### 新功能
- 结构化错误码系统：33 个（E1001-E1704），8 类别，4 严重度
- 错误日志持久化（SQLite）
- 调试包导出（日志 + DB + 配置 + 系统信息）
- 版本管理与构建元数据注入
- 5 个发布通道：Dev -> Alpha -> Beta -> RC -> Stable

---

## v0.4.0（2026-02-25）

### 新功能
- i18n 中英双语与自动检测
- 浏览器预览模式（Mock 数据）
- ErrorBoundary 组件
- 系统托盘三态图标
- Resume offload 命令

### UI/UX
- 用 SVG 替换 Emoji 图标
- Zinc/Slate 深色主题
- 状态胶囊徽章

---

## v0.3.0（2026-02-24）

### 新功能
- 级联拷贝
- 工作流预设
- 报告生成（HTML/TXT）
- SMTP 邮件通知
- `mhl-verify-cli` 独立工具

---

## v0.2.0（2026-02-23）

### 新功能
- 25+ Tauri IPC 命令
- Offload 工作流编排器
- 实时事件系统
- 设置持久化
- 前端 5 视图框架

---

## v0.1.0（2026-02-22）

### 初始版本
- 多路径并行写入拷贝引擎
- 哈希引擎：XXH64/XXH3/SHA-256/MD5（10+ GB/s）
- 检查点系统（SQLite WAL）
- ASC MHL v2.0 支持
- 按设备并发控制的 IO 调度器
- 卷发现与空间监控
