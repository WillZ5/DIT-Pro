# DIT Pro 完整测试与 Debug 报告（2026-02-28）

## 范围说明
- 本次测试与修复已**纳入当前未提交改动**一起验证（含 `src/App.tsx`、`src/views/JobsView/JobsView.tsx`、`.github/workflows/ci.yml`）。
- 未对 Notion 做任何修改。

## 执行的测试项与结果

### Frontend
- `npm run lint`：初始失败，修复后通过
- `npm run build`：通过

### Rust（src-tauri）
- `cargo fmt --all --check`：通过
- `cargo clippy --all-targets -- -D warnings`：通过
- `cargo test --lib`：通过（117 passed）
- `cargo test --tests --verbose`：通过（含 `abnormal_scenarios`、`ac_verification`、`hash_benchmark`、`real_device_test`、`stress_tests`）
- `cargo test --doc --verbose`：通过
- `cargo test --release --test hash_benchmark -- --nocapture`：通过
  - XXH64: 11.01 GB/s
  - XXH3: 29.18 GB/s
  - XXH128: 27.85 GB/s
  - SHA-256: 0.46 GB/s
  - MD5: 0.74 GB/s

### MHL Verify CLI（mhl-verify-cli）
- `cargo test`：通过（12 passed）
- `cargo build --release`：通过

## 发现的问题与处理

### 1) ESLint 扫描了 `target` 产物目录导致大量解析错误
- 根因：eslint 全局忽略只包含 `dist`，未忽略 Rust 构建输出目录
- 处理：在 `eslint.config.js` 增加 `target` 与 `**/target/**` 忽略规则

### 2) React Fast Refresh 规则报错（非组件导出）
- 触发点：`App.tsx` 导出状态函数、`i18n/index.tsx` 同时导出 Provider + Hook
- 处理：
  - 新增 `src/state/activeJobCount.ts`，将 App/Jobs 共用状态导出迁移到独立模块
  - `App.tsx` 改为订阅模块状态，不再导出非组件函数
  - `JobsView.tsx` 改为从 `state/activeJobCount` 导入
  - 对 `src/i18n/index.tsx` 在 eslint 中关闭 `react-refresh/only-export-components`（文件级）

### 3) `@typescript-eslint/no-explicit-any` 报错
- 触发点：`mockData.ts`、`tauriCompat.ts`
- 处理：
  - 移除 `window as any` 检测，改为 `\"__TAURI_INTERNALS__\" in window`
  - `safeInvoke` 参数从 `Record<string, any>` 改为 `Record<string, unknown>`
  - 为 mock invoke 参数补充类型 `MockInvokeArgs`
  - 修正 `presetData` 空值判断与类型收窄

### 4) `SystemLog` effect 规则冲突（set-state-in-effect）
- 触发点：effect 内直接调用含 `setState` 的加载函数
- 处理：改为 `queueMicrotask` 异步触发加载，避免同步 effect 触发级联渲染警告

## 本次修改文件清单
- `.github/workflows/ci.yml`
  - 调整 cargo cache 键（含架构/矩阵维度）
  - 移除 `target` 目录缓存
  - `cargo test --test '*'` 改为 `cargo test --tests`
- `eslint.config.js`
  - 新增 `target` 忽略
  - 新增 i18n 文件级 react-refresh 规则覆盖
- `src/state/activeJobCount.ts`（新增）
  - 新增全局 active job 计数状态与订阅机制
- `src/App.tsx`
  - 去除非组件导出
  - 改为订阅 `activeJobCount` 状态模块
  - 保持 JobsView 常驻挂载逻辑（你现有改动）
- `src/views/JobsView/JobsView.tsx`
  - `setActiveJobCount` 导入源迁移到状态模块
  - 移除无效 `eslint-disable` 注释
- `src/components/SystemLog.tsx`
  - effect 中改为 `queueMicrotask` 异步加载
- `src/utils/mockData.ts`
  - 去除 `any` 的 Tauri 环境检测写法
- `src/utils/tauriCompat.ts`
  - 增加类型化参数 `MockInvokeArgs`
  - 移除 `any`
  - 修复 `presetData` 的空值与类型收窄

## 当前结论
- 当前代码在本机完成了完整的前端 + Rust + CLI 测试链路，结果通过。
- 关键门禁（lint/build/fmt/clippy/tests/doc/release benchmark）均为通过状态。

## 备注
- Rust 命令中出现过 Cargo cache 的只读数据库警告（`failed to save last-use data`），不影响构建和测试结果。
