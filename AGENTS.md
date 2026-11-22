# zmpanel 项目备忘 (AGENTS.md)

## 1. 项目定位与核心目标
- **项目名称**：`zmpanel` — 面向个人多服务器（含 256MB 内存 NAT 小鸡）的极简、安全、无外联、无广告、低资源占用现代化 Linux 运维面板。
- **目标系统**：Debian、Ubuntu、Alpine Linux（生产环境采用 `musl` 静态编译单二进制）；Windows 支持本地编译运行以便开发调试与前端预览。
- **资源目标**：受控 Agent 常驻内存 8~15 MB，完整单机/主控模式 15~30 MB。
- **安全红线**：
  - 零遥测、零云端绑定、零强制登录。
  - 严禁拼接 Shell 字符串执行系统命令（使用原生 Rust API 或参数数组 `Command::new().args([...])`）。
  - 默认启用随机安全入口路径（未匹配直接返回 404，不泄露面板特征）。
  - 密码使用 Argon2id 哈希，支持 TOTP 2FA、登录失败锁定、敏感操作审计日志。

## 2. 技术栈与架构约定
- **后端**：Rust（Workspace：`zmpanel-core` + `zmpanel-server`）
  - Web/WS：`axum` + `tokio`
  - 存储：`rusqlite`（bundled 静态内置 SQLite）
  - 认证：`argon2` + `totp-rs` + HttpOnly/SameSite Cookie + Bearer Token
  - 系统采集：`sysinfo` + Linux `/proc` 扩展
  - 终端：`portable-pty` + WebSocket
  - 压缩解压：纯 Rust `zip` + `flate2` + `tar`
  - 静态资源：`rust-embed` 内嵌 `web/dist`
- **前端**：Vue 3 + TypeScript + Vite + Tailwind CSS v4 + CodeMirror 6 + xterm.js + ECharts
- **统一二进制三模式**：
  - `standalone`（单机）
  - `master`（主控，含 Web UI + 多节点管理 + 批量下发/更新）
  - `agent`（纯受控节点，无前端，主动反向连接 Master 穿透 NAT）

## 3. 功能范围（已确认）
1. 概览（CPU/内存/Swap/磁盘/实时流量与月累计流量/Swap 一键管理）
2. 网站管理（仅保留 **Go、Python、Rust、反向代理、HTML 纯静态** 5 类）
3. Docker 工作区（建项目目录 -> 传源码/Compose/命令片段部署 -> 容器状态/实时日志/终端/清理）
4. 监控与进程（历史降采样趋势图 + 进程 TOP/端口查看/结束进程）
5. 防火墙（nftables 优先、兼容 iptables/ufw，带防自锁保护）
6. 安全中心（Rust 内置轻量引擎：SSH 防爆破、端口扫描检测、异常 Web 访问拦截、SSH 加固）
7. 文件管理（CodeMirror 6 编辑器修复复制截断、上传下载、压缩解压、回收站、右键终端）
8. 终端（xterm.js + PTY、完美复制粘贴、多标签页、命令收藏夹）
9. SSL 证书中心（完全独立于建站，纯 Rust ACME HTTP-01/DNS-01、自动续期、自定义 Hook）
10. 日志中心（实时追踪系统/SSH/Nginx/项目/计划任务日志）
11. 计划任务（Cron 定时脚本/备份/清理/重启与执行历史）
12. 软件商店（声明式 TOML 清单：Nginx 含轻量安全/限流设置、Redis、PostgreSQL、MongoDB、Python `uv` 管理器）
13. 备份与跨机迁移（支持勾选网站/Docker 配置/SSL/自定义目录一键打包，支持本地下载或经反向通道直接推送备份到主控鸡仓库、换新小鸡后一键恢复迁移，支持计划任务定时推送）
14. 多节点批量管理与自更新（批量更新软件如 Nginx 漏洞修复、面板与 Agent 一键/批量自更新）
15. 系统设置与多套精致主题（6 套主题 + 自定义强调色）

## 4. 目录结构
- `crates/zmpanel-core/` — 共享配置、数据模型、错误与协议类型
- `crates/zmpanel-server/` — 主服务（Standalone / Master / Agent 入口、SQLite、认证、监控、文件、终端、路由）
- `web/` — Vue 3 + TS + Vite + Tailwind v4 前端项目（构建输出至 `web/dist`，由后端 `rust-embed` 嵌入）

## 5. 本地构建与运行约定
> 当前 Windows 开发环境沙箱中 `USERPROFILE` 被重定向且存在未启用的系统代理，需显式指定环境变量：
```powershell
$env:RUSTUP_HOME="C:\Users\zmor\.rustup"
$env:CARGO_HOME="E:\multi-cli-data\home\cargo"
$env:NO_PROXY="*"
```
- **前端开发/构建**：
  ```powershell
  cd web && pnpm install && pnpm build
  ```
- **后端测试与构建**：
  ```powershell
  cargo test
  cargo build --release
  ```
- **运行面板**：
  ```powershell
  cargo run -p zmpanel-server -- --port 18080 --data-dir ./dev-data
  ```

## 6. 当前进度与里程碑
- [x] **阶段 1（已完成）**：工程骨架（Rust + Vue3 + 6 套主题 + 6 种强调色）、动态安全入口与登录/2FA、首页概览与监控/进程/Swap、文件管理（CodeMirror 6 无损编辑复制/上传下载/压缩解压/回收站）、Web 终端（xterm.js + 原生 PTY + 完美复制粘贴 + 命令收藏夹）、系统设置与审计日志
- [x] **阶段 2（已完成）**：Docker 工作区与容器/镜像运维、网站管理（Rust/Go/Python/反代/静态 + 内置跨平台进程守护 + Nginx 站点配置生成与 CodeMirror 编辑）、软件商店声明式 TOML 框架（Nginx 轻量安全/限流配置面板 + Python `uv` 管理器）、SSL 独立证书中心（纯 Rust X.509 解析 + 自签名 + ACME 配置）、本地备份打包与 SHA256 校验安全恢复框架
- [x] **阶段 3（已完成）**：防火墙（nftables/ufw/iptables 跨系统适配 + SSH/面板端口防自锁保护）、Rust 内置轻量安全中心（SSH 防爆破/端口扫描/异常 Web 拦截 + 白名单 + SSH 加固与 `sshd -t` 回滚）、日志中心（聚合发现/关键字过滤/实时追踪/安全截断）、计划任务（纯 Rust 5 字段 Cron 调度器 + 定时自动备份轮转/脚本/HTTP/清理/站点重启）、软件商店补齐（Redis/PostgreSQL/MongoDB 参数调优与一键生成 Docker 工作区）
- [x] **阶段 4（已完成）**：Master/Agent 反向 WebSocket 加密连接（穿透 NAT 零端口映射）、主控中央备份仓库（小鸡一键拉取备份至主控 + 换新机一键推送恢复）、多节点批量下发命令与批量更新软件（如一键批量更新 Nginx 修复漏洞）、面板/Agent 自更新机制（SHA256 校验 + `--version` 自检 + `.bak` 备份原子替换）、可选 Webhook/Telegram 告警通知、Linux `musl` 静态构建脚本 (`scripts/build-linux-musl.sh`) 与 Debian/Ubuntu/Alpine 一键安装脚本 (`scripts/install.sh`)
- [x] **专项增强（已完成）**：宿主机虚拟化与 Docker 嵌套能力自动检测（[runtime_env.rs](crates/zmpanel-server/src/runtime_env.rs)，识别 Docker 鸡/LXC/OpenVZ 禁止套娃、DooD/DinD 并联动界面）；低内存极致优化（受限 Tokio 运行时免疫 32/64 核容器母鸡线程膨胀、Agent 跳过管理员 Argon2id、glibc `malloc_trim(0)` 即时归还堆页、SQLite `cache_size=-1024`，WSL Release 实测 **Agent 常驻/峰值 `9.5 MB`、Master 登录后常驻 `10.7 MB`**，单二进制 `14 MB`）

## 7. 踩过的坑与注意事项
- Windows 沙箱下 `wsl` 会返回 `Wsl/E_ACCESSDENIED`；Rust 工具链在 `C:\Users\zmor\.rustup`（只读），`CARGO_HOME` 必须指向可写目录 `E:\multi-cli-data\home\cargo`；Cargo 拉取 crates.io 需设 `NO_PROXY="*"` 绕过失效的本地 `127.0.0.1` 代理。
- `web/.npmrc` 配置了 `node-linker=hoisted`，避免 Windows 沙箱下 pnpm 符号链接导致 Vite 找不到嵌套依赖（如 `picomatch`）。
- 前端路由使用 `createWebHashHistory()`，与后端动态修改安全入口完全兼容，无需重启服务。
