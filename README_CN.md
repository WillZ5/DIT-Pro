# DIT Pro

[English](README.md) | [中文](README_CN.md)

专业影视素材拷贝引擎。基于 Tauri 2.0 + React/TypeScript + Rust 构建。

## 功能特性

- **多目标并行拷贝** -- 4MB 原子写入缓冲，支持同时写入多个目的地
- **哈希校验** -- XXH64 / XXH3 / XXH128 / SHA-256 / MD5，吞吐量 10+ GB/s
- **ASC MHL v2.0** -- 自动生成 MHL 清单，维护哈希链完整性
- **级联拷贝** -- 优先写入最快 SSD，完成后自动级联到慢速设备
- **冲突检测** -- 逐文件解决（跳过 / 覆盖 / 保留两者），支持哈希比对
- **断点续传** -- 崩溃、拔线、断电后可恢复，基于 SQLite 检查点
- **IO 调度器** -- 按设备类型控制并发（HDD 1-2 / SSD 4 / NVMe 8）
- **实时进度** -- 速度折线图、ETA 预估、逐文件状态
- **拍摄日报告** -- HTML/TXT 导出，包含逐文件哈希记录
- **邮件通知** -- SMTP 配置，拷卡完成或出错时自动通知
- **工作流预设** -- 保存和复用常用拷卡配置
- **结构化错误码** -- 33 个错误码，8 大分类，4 级严重度
- **调试包导出** -- 一键导出诊断包（日志 + 数据库 + 配置 + 系统信息）
- **国际化** -- 中文 / 英文双语

## 技术栈

| 层级 | 技术 |
|------|------|
| 应用框架 | [Tauri 2.0](https://v2.tauri.app/) |
| 前端 | React + TypeScript |
| 后端 | Rust |
| 数据库 | SQLite（WAL 模式）|
| 命令行工具 | `mhl-verify` -- 独立 MHL 链验证器 |

## 系统要求

- macOS 11.0+（Apple Silicon 原生支持）
- [Node.js](https://nodejs.org/) 18+
- [Rust](https://www.rust-lang.org/tools/install) 1.77.2+

## 开发

```bash
# 安装依赖
npm install

# 启动开发服务器（前端 HMR + Tauri 后端）
npm run dev

# 运行 Rust 测试
cargo test --manifest-path src-tauri/Cargo.toml

# 构建生产版 DMG
npx @tauri-apps/cli build
```

## 文档

- 架构说明：[English](docs/ARCHITECTURE.md) | [中文](docs/ARCHITECTURE_CN.md)
- 更新日志：[English](docs/CHANGELOG.md) | [中文](docs/CHANGELOG_CN.md)

## 项目结构

```
src/                        # React 前端
  views/                    # 5 个主视图：任务、卷宗、预设、报告、设置
  i18n/                     # 国际化（中/英）
src-tauri/                  # Rust 后端
  src/
    copy_engine/            # 多路径并行拷贝 + 原子写入
      atomic_writer.rs      # 原子 .tmp -> 重命名，Drop 自动清理
    hash_engine/            # XXH64 / XXH3 / XXH128 / SHA-256 / MD5
    workflow/               # 拷卡编排器（扫描 -> 拷贝 -> 校验 -> MHL -> 通知）
    mhl/                    # ASC MHL v2.0 清单生成
    checkpoint/             # 崩溃恢复与断点续传
    volume/                 # 设备发现与空间监控
    io_scheduler/           # 按设备类型并发控制
    db/                     # SQLite 建表与迁移
    config.rs               # JSON 配置持久化
    preset.rs               # 工作流预设管理
    report.rs               # 拍摄日报告与任务报告（HTML/TXT）
    notify/                 # 邮件通知系统（SMTP）
    tray.rs                 # 系统托盘（空闲/活跃/错误三态）
    error.rs                # 结构化错误码（E1001-E1704）
    error_log.rs            # 错误日志持久化
    debug_bundle.rs         # 一键诊断包导出
    version.rs              # 语义化版本与发布通道
    commands.rs             # 44 个 Tauri IPC 命令
mhl-verify-cli/             # 独立 MHL 链验证命令行工具
```

## 许可证

MIT 许可证。详见 [LICENSE](LICENSE)。
