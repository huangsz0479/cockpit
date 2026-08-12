# Cockpit

一个面向日常开发与运维的跨平台桌面数据库客户端。

Cockpit 使用 Tauri 2、Vue 3 和 Rust 构建，支持 MySQL、MariaDB、PostgreSQL 与 SQLite。它把连接管理、SQL 编辑、数据维护、结构设计、导入导出和基础运维集中在一个本地应用中，并在执行高风险操作前提供明确的安全确认。

> 当前版本：`0.1.5`

## 支持的数据库

| 数据库 | 连接方式 | 主要能力 |
| --- | --- | --- |
| MySQL / MariaDB | 直连、TLS、SSH Agent 或私钥隧道 | 查询、数据编辑、对象管理、结构设计、运维与备份恢复 |
| PostgreSQL | 直连、TLS | 查询、数据编辑、对象管理、运维与备份恢复 |
| SQLite | 本地数据库文件 | 查询、数据编辑、对象管理与备份恢复 |

PostgreSQL 暂不支持 SSH 隧道。不同数据库对元数据和管理功能的支持程度可能不同，界面会根据当前驱动显示可用操作。

## 核心能力

### 连接与对象浏览

- 保存、分组、测试、连接和断开数据库连接
- 使用系统凭据库存储密码，支持只读连接与生产环境标记
- 浏览数据库、模式、表、视图、函数、过程和触发器
- 查看列、索引、外键、DDL 与对象定义，支持服务端搜索和分页

### SQL 工作区

- 基于 CodeMirror 的多标签 SQL 编辑器，自动匹配数据库方言
- SQL 补全、格式化、参数化执行、当前语句或选区执行
- 执行计划、查询取消、超时控制、多结果集和数据库端分页
- 查询历史、收藏、片段库、最近关闭标签恢复与工作区自动恢复
- 打开、保存和另存 SQL 文件

### 数据与结构管理

- 分页浏览、筛选、排序、搜索和数值概要
- 单行或批量新增、编辑和删除，支持显式提交与回滚
- 使用唯一键和原值校验检测并发修改
- JSON 格式化、二进制预览与保存、空间值查看，以及高精度数值显示
- MySQL / MariaDB 可视化建表与 `ALTER SQL` 预览
- 数据库结构对比、迁移 SQL 与回滚 SQL 生成

### 导入、导出与运维

- 导入 CSV、XLSX、XLS、XLSB 和 SQL，支持字段映射、冲突策略、进度与取消
- 导出 TXT、SQL、CSV 和 XLSX，可选择当前页、完整查询或整表
- 按数据库方言备份结构与数据，支持 Gzip、AES-256-GCM 加密和 SHA-256 校验
- 查看并终止会话，检查服务器状态、变量、锁等待和用户信息
- MySQL / MariaDB 可查看复制与 binlog 信息

## 安全与隐私

Cockpit 默认采用保守策略处理数据库写入：

- 只读连接会在 Rust 驱动层拦截写操作
- `UPDATE`、`DELETE`、DDL 和无法可靠分类的 SQL 默认需要确认
- 无 `WHERE` 的 `UPDATE` / `DELETE` 以及 `DROP` / `TRUNCATE` 会标记为高风险
- 行更新和删除必须具有主键或唯一键，并检查原值是否已被其他会话修改
- CSV 文本默认防护电子表格公式注入
- 密码由操作系统凭据库保管，不写入 Cockpit 的本地项目数据库
- 诊断日志会对敏感连接信息进行脱敏

## 快速开始

### 环境要求

- Node.js `20.19+`
- npm `10+`
- Rust `1.88+`
- 当前平台所需的 Tauri 2 系统依赖

### 启动桌面应用

```bash
npm install
npm run dev:tauri
```

首次构建需要下载前端和 Rust 依赖，耗时会比后续启动更长。

### 构建安装包

```bash
npm run tauri build
```

构建产物由 Tauri 写入 `target/release/bundle/`。具体格式取决于当前操作系统：Windows 为 NSIS / MSI，macOS 为 DMG，Linux 为 AppImage / DEB。

## 开发与验证

常用命令：

| 命令 | 用途 |
| --- | --- |
| `npm run dev:tauri` | 启动 Vite 与 Tauri 开发环境 |
| `npm test` | 运行前端单元测试 |
| `npm run build` | 执行 TypeScript 检查并构建前端 |
| `cargo test --workspace` | 运行 Rust 工作区测试 |
| `cargo fmt --all -- --check` | 检查 Rust 格式 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 检查 Rust 代码质量 |

提交前建议执行完整检查：

```bash
npm test
npm run build
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

### MySQL 集成测试

CI 会使用 MySQL 5.7、8.0 和 8.4 验证元数据读取、分页、行写入与事务回滚。本地运行时，先通过 `COCKPIT_TEST_MYSQL_*` 环境变量配置专用测试实例，然后执行：

```bash
cargo test -p cockpit-mysql --test mysql_integration -- --ignored
```

集成测试会在指定数据库中创建并删除 `cockpit_matrix` 表，请勿连接生产数据库。

## 工程结构

```text
Cockpit/
├── src/                         Vue 3 界面、Pinia 状态与前端测试
├── src-tauri/                   Tauri 桌面入口、命令、会话与凭据管理
├── crates/
│   ├── cockpit-core/            公共模型、驱动接口、安全规则与交换格式
│   ├── cockpit-mysql/           MySQL / MariaDB 驱动、TLS 与 SSH
│   ├── cockpit-postgres/        PostgreSQL 驱动与 TLS
│   └── cockpit-sqlite/          SQLite 驱动与本地文件访问
├── .github/workflows/           持续集成与多平台发布流程
├── package.json                 前端依赖和开发脚本
└── Cargo.toml                   Rust 工作区配置
```

前端通过 Tauri command 调用 Rust 后端；`cockpit-core` 定义统一驱动接口，各数据库 crate 负责方言、元数据、查询和行操作的具体实现。

## 发布流程

- 每次 push 和 pull request 都会在 macOS、Windows 上运行前端与 Rust 检查，并在 Linux 上执行 MySQL 版本矩阵测试
- `release` 分支通过 CI 后，会生成未签名的三平台 Preview 安装包并发布到 `preview-v<版本号>` 预发布
- 推送 `v*` 标签或手动运行 `Release bundles` 工作流，会构建正式版本
- macOS 公证、Windows 签名和 Tauri 更新签名需要在仓库中单独配置相应凭据；凭据不应提交到版本库

## 已知限制

- PostgreSQL 目前仅支持直连与 TLS，不支持 SSH 隧道
- 更新检查只读取用户配置的 HTTPS JSON 清单并提示，不会自动下载或安装新版本
- 定时备份仅在 Cockpit 运行期间生效，退出应用后不会在系统后台继续执行
- CSV 和 Excel 导入会在提交前将已解析数据保留在内存中，超大文件建议分批处理

## 许可证

本项目基于 [Apache License 2.0](LICENSE) 发布。
